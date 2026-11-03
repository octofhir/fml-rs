//! Running a StructureMap.
//!
//! Follows the HL7 reference engine (`StructureMapUtilities.transform` in org.hl7.fhir.core):
//! groups run their extended group first, each rule runs once per source item that passes its
//! filters, targets are created or reused (repeating elements get a new value, `share` reuses one),
//! dependent groups receive variables by position, and a simple copy without a dependent group is
//! completed by the default group for the source and target types.
//!
//! Where the Mapping Language IG says more than the reference engine does, the engine follows the
//! IG: a rule may have several sources and then runs once per combination of their items; named
//! groups and nested rules after `then` both run, the groups first; a source's `min..max` is
//! enforced; `evaluate` with several values makes one target per value in a repeating element; a
//! group name found in several imported maps is an error; `cast` takes `FHIR.` and `System.`
//! types and checks the value.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use fml_check::definitions::type_url;
use fml_check::{Definitions, Lookup, Type, Types};
use fml_model::model::{
    Dependent, Group, Parameter, ParameterValue, Rule, Source, StructureMap, Target,
};
use fml_model::{SOURCE_ELEMENT_EXTENSION, TARGET_ELEMENT_EXTENSION};
use serde_json::{Value, json};

use crate::expressions::{Expressions, Item, bind_variables, path_start_names};
use crate::ids::{IdGenerator, RandomIds};
use crate::terminology::{ConceptMapResolver, NoConceptMaps, TranslateService, translate};
use crate::trace::{Phase, Trace, TraceEvent, TraceStatus, TraceVariable};
use crate::tree::{NodeId, Tree};
use crate::{datetime, transforms};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// A failed transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    /// The map the failure is in: its canonical URL.
    pub map: Option<String>,
    /// Where in that map, as a FHIRPath location: `StructureMap.group[0].rule[2].source[0]`.
    pub position: Option<String>,
    /// With tracing on, the events recorded up to the failure (status `stopped`).
    pub trace: Option<Box<Trace>>,
}

/// Why a transform failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The map cannot be run on this input.
    Failed,
    /// It ran past one of the [`Limits`].
    Limit,
    /// The caller cancelled it.
    Cancelled,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Failed,
            message: message.into(),
            map: None,
            position: None,
            trace: None,
        }
    }

    fn limit(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Limit,
            ..Self::new(message)
        }
    }

    /// Records where the error happened, unless a deeper part already did.
    fn at(mut self, map: &StructureMap, position: &str) -> Self {
        if self.position.is_none() {
            self.map = map.url.clone();
            self.position = Some(position.to_owned());
        }
        self
    }

    fn in_rule(rule: &Rule, message: impl fmt::Display) -> Self {
        match &rule.name {
            Some(name) => Self::new(format!("rule `{name}`: {message}")),
            None => Self::new(message.to_string()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// Finds the maps a map imports, by canonical URL or `*` pattern.
pub trait MapResolver {
    fn resolve(&self, url: &str) -> Vec<Arc<StructureMap>>;
}

/// No imported maps.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMaps;

impl MapResolver for NoMaps {
    fn resolve(&self, _url: &str) -> Vec<Arc<StructureMap>> {
        Vec::new()
    }
}

/// A fixed set of maps, matched by URL; `*` in an import matches any run of characters.
#[derive(Debug, Default, Clone)]
pub struct MapSet {
    maps: Vec<Arc<StructureMap>>,
}

impl MapSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, map: StructureMap) {
        self.maps.push(Arc::new(map));
    }
}

impl MapResolver for MapSet {
    fn resolve(&self, url: &str) -> Vec<Arc<StructureMap>> {
        self.maps
            .iter()
            .filter(|map| {
                map.url
                    .as_deref()
                    .is_some_and(|candidate| match url.split_once('*') {
                        Some((prefix, suffix)) => {
                            candidate.len() >= prefix.len() + suffix.len()
                                && candidate.starts_with(prefix)
                                && candidate.ends_with(suffix)
                        }
                        None => candidate == url,
                    })
            })
            .cloned()
            .collect()
    }
}

/// How much a transform may do before it stops with an [`ErrorKind::Limit`] error, and how much
/// of its trace is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Groups calling groups.
    pub max_depth: usize,
    /// Rules run, counting each run of a rule once.
    pub max_rules: u64,
    /// Values created: target elements and intermediate values.
    pub max_values: usize,
    /// Trace events kept; later events are left out and the trace is `truncated`.
    pub max_trace_events: usize,
    /// Roughly the size of the kept trace events as JSON, in bytes.
    pub max_trace_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 512,
            max_rules: 20_000_000,
            max_values: 2_000_000,
            max_trace_events: 10_000,
            max_trace_bytes: 8 * 1024 * 1024,
        }
    }
}

/// The result of a transform.
#[derive(Debug, Clone, PartialEq)]
pub struct Transformed {
    /// The target, as FHIR JSON.
    pub target: Value,
    /// Messages from `log (…)` clauses.
    pub log: Vec<String>,
    /// Values left out of the target, such as a code a ConceptMap does not know (the reference
    /// engine leaves those unset too).
    pub warnings: Vec<String>,
    /// Resources the map created outside the target (`create('Observation') as obs` with no
    /// target element) and did not put into it: the `produced` structures of the map.
    pub produced: Vec<Value>,
    /// The debug trace, when tracing is on ([`Engine::with_trace`]).
    pub trace: Option<Trace>,
}

/// Runs StructureMaps.
pub struct Engine<'a, X> {
    definitions: &'a dyn Definitions,
    expressions: X,
    maps: &'a dyn MapResolver,
    concept_maps: &'a dyn ConceptMapResolver,
    /// `$translate` for ConceptMaps the resolver does not have.
    translator: Option<&'a dyn TranslateService>,
    ids: &'a dyn IdGenerator,
    limits: Limits,
    cancel: Option<&'a AtomicBool>,
    trace: bool,
    /// Group lookups, which look through every imported map: the maps do not change between
    /// transforms, so each answer is computed once.
    groups: Mutex<GroupCache>,
}

/// A target type with its FHIR type name.
type TypedTarget = (Type, Option<String>);

/// Cached lookups of groups in imported maps, by the importing map's URL and imports. Groups of
/// the map being run are looked up in it each time, so a changed map with the same URL is not
/// answered from the cache.
#[derive(Default)]
struct GroupCache {
    by_name: HashMap<(String, String), Found>,
    by_types: HashMap<(String, String, String), Option<GroupRef>>,
    default_target: HashMap<(String, String), Option<TypedTarget>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Input,
    Output,
    Shared,
}

/// Variables in scope: later entries hide earlier ones.
#[derive(Debug, Clone, Default)]
struct Vars {
    entries: Vec<(Mode, String, NodeId)>,
}

impl Vars {
    fn add(&mut self, mode: Mode, name: &str, node: NodeId) {
        self.entries.push((mode, name.to_owned(), node));
    }

    fn get(&self, mode: Mode, name: &str) -> Option<NodeId> {
        self.entries
            .iter()
            .rev()
            .find(|(m, n, _)| *m == mode && n == name)
            .map(|(_, _, node)| *node)
    }

    /// An input, or an output used as an input, as the reference engine allows.
    fn value(&self, name: &str) -> Option<NodeId> {
        self.get(Mode::Input, name)
            .or_else(|| self.get(Mode::Output, name))
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|(mode, _, _)| *mode != Mode::Shared)
            .map(|(_, name, _)| name.as_str())
    }
}

/// A group looked up by name.
#[derive(Clone)]
enum Found {
    One(GroupRef),
    None,
    /// The URLs of the imported maps that each have the group.
    Several(Vec<String>),
}

/// A group of a map.
#[derive(Clone)]
struct GroupRef {
    map: Arc<StructureMap>,
    index: usize,
}

impl GroupRef {
    fn group(&self) -> &Group {
        &self.map.group[self.index]
    }
}

/// State of one transform.
#[derive(Default)]
struct Run {
    tree: Tree,
    log: Vec<String>,
    warnings: Vec<String>,
    depth: usize,
    /// Recorded events, when tracing.
    trace: Option<Vec<TraceEvent>>,
    trace_depth: usize,
    /// Resources created outside the target.
    produced: Vec<NodeId>,
    /// Evaluated `let` constants, by map URL and name.
    constants: HashMap<(String, String), Vec<NodeId>>,
    /// Constants being evaluated, to report one that refers to itself.
    evaluating: HashSet<(String, String)>,
    /// Rules run so far.
    rules: u64,
    /// Roughly the size of the recorded trace events, in bytes.
    trace_bytes: usize,
    /// Why later trace events were left out.
    trace_truncated: Option<String>,
    /// Elements written by a `single` target: element, owner and target.
    single: HashSet<(NodeId, String, usize)>,
    /// The `first` and `last` values of repeating elements, by owner and element.
    order: HashMap<(NodeId, String), TargetOrder>,
}

/// Where the `first` and `last` targets of one repeating element put their values.
#[derive(Default)]
struct TargetOrder {
    first: Option<usize>,
    first_count: usize,
    last: Option<usize>,
    last_values: Vec<NodeId>,
}
const CORE: &str = "http://hl7.org/fhir/StructureDefinition/";

impl<'a, X: Expressions> Engine<'a, X> {
    pub fn new(definitions: &'a dyn Definitions, expressions: X) -> Self {
        Self {
            definitions,
            expressions,
            maps: &NoMaps,
            concept_maps: &NoConceptMaps,
            translator: None,
            ids: &RandomIds,
            limits: Limits::default(),
            cancel: None,
            trace: false,
            groups: Mutex::new(GroupCache::default()),
        }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, GroupCache> {
        self.groups.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a debug trace of each transform: see [`Trace`].
    pub fn with_trace(mut self, trace: bool) -> Self {
        self.trace = trace;
        self
    }

    /// Where `translate` finds ConceptMaps that are not contained in the map.
    pub fn with_concept_maps(mut self, concept_maps: &'a dyn ConceptMapResolver) -> Self {
        self.concept_maps = concept_maps;
        self
    }

    /// A terminology service for `translate` with ConceptMaps neither the map nor the resolver
    /// holds.
    pub fn with_translator(mut self, translator: &'a dyn TranslateService) -> Self {
        self.translator = Some(translator);
        self
    }

    /// How much a transform may do; see [`Limits`].
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// A flag the caller sets to stop running transforms; they fail with
    /// [`ErrorKind::Cancelled`] at the next rule.
    pub fn with_cancellation(mut self, cancel: &'a AtomicBool) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Where `uuid()` and new resource ids come from; random by default.
    pub fn with_ids(mut self, ids: &'a dyn IdGenerator) -> Self {
        self.ids = ids;
        self
    }

    /// Where the maps named in `imports` come from.
    pub fn with_maps(mut self, maps: &'a dyn MapResolver) -> Self {
        self.maps = maps;
        self
    }

    fn types(&self) -> Types<'a> {
        Types::new(self.definitions)
    }

    /// Transforms `source` with the map's first group; the target is created from the type of
    /// that group's target parameter.
    pub async fn transform(&self, map: &StructureMap, source: &Value) -> Result<Transformed> {
        self.run(map, source, None).await
    }

    /// Transforms `source` into an existing `target`.
    pub async fn transform_into(
        &self,
        map: &StructureMap,
        source: &Value,
        target: &Value,
    ) -> Result<Transformed> {
        self.run(map, source, Some(target)).await
    }

    async fn run(
        &self,
        map: &StructureMap,
        source: &Value,
        target: Option<&Value>,
    ) -> Result<Transformed> {
        let map = Arc::new(map.clone());
        let group = map
            .group
            .first()
            .ok_or_else(|| Error::new("the map has no groups"))?;
        let input = |mode: &str| group.input.iter().find(|input| input.mode == mode);
        let source_input =
            input("source").ok_or_else(|| Error::new("the first group has no source"))?;
        let target_input =
            input("target").ok_or_else(|| Error::new("the first group has no target"))?;

        let mut run = Run {
            trace: self.trace.then(Vec::new),
            ..Run::default()
        };
        let types = self.types();
        let source_type = self.input_type(&map, source_input.type_name.as_deref());
        let source_node = run.tree.load(source, source_type, &types);
        if run.trace.is_some() {
            // The trace says where an input variable is in the input (debug.md, variable.path).
            let root_name = run
                .tree
                .fhir_type(source_node)
                .unwrap_or_else(|| "%resource".to_owned());
            run.tree.record_paths(source_node, &root_name);
        }
        let target_type = self.input_type(&map, target_input.type_name.as_deref());
        let target_node = match target {
            Some(target) => run.tree.load(target, target_type, &types),
            None => {
                if matches!(target_type, Type::Unknown) {
                    return Err(Error::new(format!(
                        "cannot create the target: type `{}` is not known",
                        target_input.type_name.as_deref().unwrap_or("?")
                    )));
                }
                run.tree.create(target_type, None)
            }
        };
        let mut vars = Vars::default();
        vars.add(Mode::Input, &source_input.name, source_node);
        vars.add(Mode::Output, &target_input.name, target_node);
        let entry = GroupRef {
            map: map.clone(),
            index: 0,
        };
        if let Err(mut error) = self.execute_group(&mut run, entry, vars, true).await {
            let truncated = run.trace_truncated.take();
            error.trace = run.trace.take().map(|events| {
                Box::new(Trace {
                    status: match truncated {
                        Some(_) => TraceStatus::Truncated,
                        None => TraceStatus::Stopped,
                    },
                    note: truncated,
                    events,
                })
            });
            return Err(error);
        }
        let in_target = run.tree.reachable(target_node);
        let produced = run
            .produced
            .iter()
            .filter(|node| !in_target.contains(node))
            .map(|&node| run.tree.to_json(node))
            .collect();
        Ok(Transformed {
            target: run.tree.to_json(target_node),
            log: run.log,
            warnings: run.warnings,
            produced,
            trace: run.trace.map(|events| Trace {
                status: match run.trace_truncated {
                    Some(_) => TraceStatus::Truncated,
                    None => TraceStatus::Complete,
                },
                note: run.trace_truncated,
                events,
            }),
        })
    }

    /// The type a group parameter or `create` names: a `uses` alias, a URL, or a type name.
    fn input_type(&self, map: &StructureMap, name: Option<&str>) -> Type {
        let Some(name) = name else {
            return Type::Unknown;
        };
        self.types().structure(&self.type_name_url(map, name, CORE))
    }

    fn type_name_url(&self, map: &StructureMap, name: &str, context: &str) -> String {
        if let Some(structure) = map
            .structure
            .iter()
            .find(|s| s.alias.as_deref() == Some(name))
        {
            return structure.url.clone();
        }
        type_url(name, context)
    }

    // ---- Groups -----------------------------------------------------------------------------

    fn execute_group<'r>(
        &'r self,
        run: &'r mut Run,
        group: GroupRef,
        vars: Vars,
        at_root: bool,
    ) -> BoxFuture<'r, Result<()>> {
        Box::pin(async move {
            run.depth += 1;
            if run.depth > self.limits.max_depth {
                return Err(Error::limit(format!(
                    "groups nest more than {} deep; is a group calling itself without end?",
                    self.limits.max_depth
                )));
            }
            let map = group.map.clone();
            let definition = group.group();
            let position = format!("StructureMap.group[{}]", group.index);
            self.record(run, Phase::Enter, &map, &position, &vars, Vec::new());
            if let Some(extends) = &definition.extends {
                let base = self.group_by_name(&map, extends)?;
                self.execute_group(run, base, vars.clone(), false).await?;
            }
            let mut vars = vars;
            for (index, rule) in definition.rule.iter().enumerate() {
                let rule_position = format!("{position}.rule[{index}]");
                self.execute_rule(run, &group, rule, &rule_position, &mut vars, at_root)
                    .await?;
            }
            self.record(run, Phase::Exit, &map, &position, &vars, Vec::new());
            run.depth -= 1;
            Ok(())
        })
    }

    /// The group `name`, in the map or the maps it imports.
    fn group_by_name(&self, map: &Arc<StructureMap>, name: &str) -> Result<GroupRef> {
        if let Some(index) = map.group.iter().position(|g| g.name == name) {
            return Ok(GroupRef {
                map: map.clone(),
                index,
            });
        }
        let key = (imports_key(map), name.to_owned());
        let cached = self.cache().by_name.get(&key).cloned();
        let found = match cached {
            Some(found) => found,
            None => {
                let found = self.find_imported_group_by_name(map, name);
                self.cache().by_name.insert(key, found.clone());
                found
            }
        };
        match found {
            Found::One(group) => Ok(group),
            Found::None => Err(Error::new(format!(
                "no group `{name}` in {} or the maps it imports",
                map.url.as_deref().unwrap_or("the map")
            ))),
            Found::Several(urls) => Err(Error::new(format!(
                "group `{name}` is in several imported maps ({}); a group name must be unique",
                urls.join(", ")
            ))),
        }
    }

    /// The group in exactly one of the maps `map` imports (fml.md: a group name must be unique
    /// among the imported maps).
    fn find_imported_group_by_name(&self, map: &Arc<StructureMap>, name: &str) -> Found {
        let mut found: Vec<GroupRef> = Vec::new();
        for import in &map.import {
            for imported in self.maps.resolve(import) {
                if imported.url == map.url || found.iter().any(|g| g.map.url == imported.url) {
                    continue;
                }
                if let Some(index) = imported.group.iter().position(|g| g.name == name) {
                    found.push(GroupRef {
                        map: imported.clone(),
                        index,
                    });
                }
            }
        }
        match found.len() {
            0 => Found::None,
            1 => Found::One(found.remove(0)),
            _ => Found::Several(
                found
                    .iter()
                    .map(|g| g.map.url.clone().unwrap_or_default())
                    .collect(),
            ),
        }
    }

    /// The default group for a source and target type: one with a type mode whose parameter
    /// types match.
    fn group_by_types(
        &self,
        map: &Arc<StructureMap>,
        source: Option<&str>,
        target: Option<&str>,
    ) -> Option<GroupRef> {
        let (source, target) = (source?, target?);
        if let Some(index) = self.default_group_index(map, source, target) {
            return Some(GroupRef {
                map: map.clone(),
                index,
            });
        }
        let key = (imports_key(map), source.to_owned(), target.to_owned());
        if let Some(found) = self.cache().by_types.get(&key) {
            return found.clone();
        }
        let found = self.find_imported_group_by_types(map, source, target);
        self.cache().by_types.insert(key, found.clone());
        found
    }

    /// The default group of `map` itself for the types.
    fn default_group_index(&self, map: &StructureMap, source: &str, target: &str) -> Option<usize> {
        map.group.iter().position(|group| {
            group.type_mode.is_some()
                && matches!(group.input.as_slice(), [s, t]
                    if s.mode == "source" && t.mode == "target"
                    && self.stated_type_is(map, s.type_name.as_deref(), source)
                    && self.stated_type_is(map, t.type_name.as_deref(), target))
        })
    }

    fn find_imported_group_by_types(
        &self,
        map: &Arc<StructureMap>,
        source: &str,
        target: &str,
    ) -> Option<GroupRef> {
        for import in &map.import {
            for imported in self.maps.resolve(import) {
                if imported.url == map.url {
                    continue;
                }
                if let Some(index) = self.default_group_index(&imported, source, target) {
                    return Some(GroupRef {
                        map: imported,
                        index,
                    });
                }
            }
        }
        None
    }

    /// The type a source type filter names: through a `uses` alias to the structure's type
    /// (`: CodeableConceptR4` is `CodeableConcept`), or the name as written.
    fn stated_type_name(&self, map: &StructureMap, stated: &str) -> String {
        map.structure
            .iter()
            .find(|s| s.alias.as_deref() == Some(stated))
            .and_then(|s| self.definitions.structure(&s.url))
            .map_or_else(|| stated.to_owned(), |s| s.type_name.clone())
    }

    fn stated_type_is(&self, map: &StructureMap, stated: Option<&str>, actual: &str) -> bool {
        let Some(stated) = stated else { return false };
        if stated == actual {
            return true;
        }
        let url = self.type_name_url(map, stated, CORE);
        self.definitions
            .structure(&url)
            .is_some_and(|s| s.type_name == actual)
    }

    // ---- Rules ------------------------------------------------------------------------------

    fn execute_rule<'r>(
        &'r self,
        run: &'r mut Run,
        group: &'r GroupRef,
        rule: &'r Rule,
        position: &'r str,
        vars: &'r mut Vars,
        at_root: bool,
    ) -> BoxFuture<'r, Result<()>> {
        Box::pin(async move {
            self.check_limits(run)?;
            self.record(run, Phase::Enter, &group.map, position, vars, Vec::new());
            let mut bindings = vec![vars.clone()];
            for (index, source) in rule.source.iter().enumerate() {
                let logged = run.log.len();
                let mut next = Vec::new();
                for binding in &bindings {
                    next.extend(
                        self.process_source(run, group, rule, source, binding)
                            .await
                            .map_err(|e| {
                                e.at(&group.map, &format!("{position}.source[{index}]"))
                            })?,
                    );
                }
                bindings = next;
                if run.trace.is_some() {
                    let messages = run.log[logged..].to_vec();
                    let shown = bindings.first().cloned().unwrap_or_else(|| vars.clone());
                    let step = format!("{position}.source[{index}]");
                    self.record(run, Phase::Step, &group.map, &step, &shown, messages);
                }
            }
            let source_variable = match rule.source.as_slice() {
                [single] => single.variable.as_deref(),
                _ => None,
            };
            for mut binding in bindings {
                for (index, target) in rule.target.iter().enumerate() {
                    self.process_target(
                        run,
                        group,
                        rule,
                        target,
                        &mut binding,
                        vars,
                        source_variable,
                        at_root,
                    )
                    .await
                    .map_err(|e| e.at(&group.map, &format!("{position}.target[{index}]")))?;
                    if run.trace.is_some() {
                        let step = format!("{position}.target[{index}]");
                        self.record(run, Phase::Step, &group.map, &step, &binding, Vec::new());
                    }
                }
                // The named groups run first, then the nested rules (fml.md, "Dependent Rules").
                for dependent in &rule.dependent {
                    self.execute_dependency(run, group, rule, dependent, &binding)
                        .await
                        .map_err(|e| e.at(&group.map, position))?;
                }
                for (index, child) in rule.rule.iter().enumerate() {
                    let child_position = format!("{position}.rule[{index}]");
                    self.execute_rule(run, group, child, &child_position, &mut binding, false)
                        .await?;
                }
                if rule.dependent.is_empty()
                    && rule.rule.is_empty()
                    && let Some((source, target)) = simple_copy(rule)
                {
                    self.default_copy(run, group, rule, source, target, &binding)
                        .await
                        .map_err(|e| e.at(&group.map, position))?;
                }
            }
            self.record(run, Phase::Exit, &group.map, position, vars, Vec::new());
            Ok(())
        })
    }

    /// Stops the transform when the caller cancelled it or it ran past its limits.
    fn check_limits(&self, run: &mut Run) -> Result<()> {
        if self
            .cancel
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return Err(Error {
                kind: ErrorKind::Cancelled,
                ..Error::new("the transform was cancelled")
            });
        }
        run.rules += 1;
        if run.rules > self.limits.max_rules {
            return Err(Error::limit(format!(
                "the transform ran more than {} rules",
                self.limits.max_rules
            )));
        }
        if run.tree.len() > self.limits.max_values {
            return Err(Error::limit(format!(
                "the transform created more than {} values",
                self.limits.max_values
            )));
        }
        Ok(())
    }

    /// Records a trace event, when tracing.
    fn record(
        &self,
        run: &mut Run,
        phase: Phase,
        map: &StructureMap,
        position: &str,
        vars: &Vars,
        messages: Vec<String>,
    ) {
        if run.trace.is_none() {
            return;
        }
        if phase == Phase::Exit {
            run.trace_depth = run.trace_depth.saturating_sub(1);
        }
        if run.trace_truncated.is_some() {
            if phase == Phase::Enter {
                run.trace_depth += 1;
            }
            return;
        }
        let mut seen = HashSet::new();
        let mut variables: Vec<TraceVariable> = vars
            .entries
            .iter()
            .rev()
            .filter(|(mode, name, _)| *mode != Mode::Shared && seen.insert(name.clone()))
            .map(|(mode, name, node)| TraceVariable {
                name: name.clone(),
                context_type: match mode {
                    Mode::Input => "input",
                    _ => "output",
                }
                .to_owned(),
                value: run.tree.to_json(*node),
                data_type: self.item(run, *node).type_name,
                path: run.tree.path(*node).map(str::to_owned),
            })
            .collect();
        variables.reverse();
        let event = TraceEvent {
            phase,
            depth: run.trace_depth,
            source: map.url.clone(),
            position: position.to_owned(),
            variables,
            messages,
        };
        if run.trace_truncated.is_none() {
            let size = event.approximate_size();
            let count = run.trace.as_ref().map_or(0, Vec::len);
            if count >= self.limits.max_trace_events {
                run.trace_truncated = Some(format!(
                    "the trace keeps its first {} events",
                    self.limits.max_trace_events
                ));
            } else if run.trace_bytes + size > self.limits.max_trace_bytes {
                run.trace_truncated = Some(format!(
                    "the trace keeps its first {count} events, about {} bytes",
                    run.trace_bytes
                ));
            } else if let Some(events) = &mut run.trace {
                run.trace_bytes += size;
                events.push(event);
            }
        }
        if phase == Phase::Enter {
            run.trace_depth += 1;
        }
    }

    /// `src.a as s -> tgt.a as t` with nothing else to do: the default group for the two types
    /// maps one into the other, or the value is copied when the types are the same.
    async fn default_copy(
        &self,
        run: &mut Run,
        group: &GroupRef,
        rule: &Rule,
        source: &str,
        target: &str,
        vars: &Vars,
    ) -> Result<()> {
        let source = vars
            .get(Mode::Input, source)
            .ok_or_else(|| Error::in_rule(rule, format!("no source variable `{source}`")))?;
        let target = vars
            .get(Mode::Output, target)
            .ok_or_else(|| Error::in_rule(rule, format!("no target variable `{target}`")))?;
        let source_type = run.tree.fhir_type(source);
        let target_type = run.tree.fhir_type(target);
        if let Some(default) =
            self.group_by_types(&group.map, source_type.as_deref(), target_type.as_deref())
        {
            let inputs = &default.group().input;
            let mut call = Vars::default();
            call.add(Mode::Input, &inputs[0].name, source);
            call.add(Mode::Output, &inputs[1].name, target);
            return self.execute_group(run, default, call, false).await;
        }
        if source_type == target_type || target_type.is_none() {
            run.tree.assign(target, source);
            return Ok(());
        }
        Err(Error::in_rule(
            rule,
            format!(
                "no default group maps {} to {}",
                source_type.as_deref().unwrap_or("?"),
                target_type.as_deref().unwrap_or("?")
            ),
        ))
    }

    async fn execute_dependency(
        &self,
        run: &mut Run,
        group: &GroupRef,
        rule: &Rule,
        dependent: &Dependent,
        vars: &Vars,
    ) -> Result<()> {
        let callee = self.group_by_name(&group.map, &dependent.name)?;
        let inputs = &callee.group().input;
        if inputs.len() != dependent.parameter.len() {
            return Err(Error::in_rule(
                rule,
                format!(
                    "group `{}` takes {} arguments, the call passes {}",
                    dependent.name,
                    inputs.len(),
                    dependent.parameter.len()
                ),
            ));
        }
        let mut call = Vars::default();
        for (input, parameter) in inputs.iter().zip(&dependent.parameter) {
            let name = parameter_name(parameter);
            let (mode, node) = if input.mode == "source" {
                (Mode::Input, vars.value(&name))
            } else {
                (Mode::Output, vars.get(Mode::Output, &name))
            };
            let node = node.ok_or_else(|| {
                Error::in_rule(
                    rule,
                    format!(
                        "`{}` passes `{name}` as {} `{}`, but it has no value",
                        dependent.name, input.mode, input.name
                    ),
                )
            })?;
            call.add(mode, &input.name, node);
        }
        self.execute_group(run, callee, call, false).await
    }

    // ---- Sources ----------------------------------------------------------------------------

    async fn process_source(
        &self,
        run: &mut Run,
        group: &GroupRef,
        rule: &Rule,
        source: &Source,
        vars: &Vars,
    ) -> Result<Vec<Vars>> {
        let context = vars.get(Mode::Input, &source.context).ok_or_else(|| {
            Error::in_rule(
                rule,
                format!("unknown source variable `{}`", source.context),
            )
        })?;
        let mut items = vec![context];
        for element in element_path(
            source.element.as_deref(),
            &source.extension,
            SOURCE_ELEMENT_EXTENSION,
        ) {
            items = items
                .iter()
                .flat_map(|&item| run.tree.children(item, &element))
                .collect();
        }
        check_cardinality(rule, source, items.len())?;
        // The type filter selects the exact type; the default stands in when nothing is left.
        if let Some(type_name) = &source.type_name {
            let wanted = self.stated_type_name(&group.map, type_name);
            items.retain(|&item| run.tree.fhir_type(item).as_deref() == Some(wanted.as_str()));
        }
        if items.is_empty()
            && let Some(default) = &source.default_value
        {
            items = self.evaluate(run, group, default, None, vars).await?;
        }
        let with_item = |item: NodeId| {
            let mut bound = vars.clone();
            if let Some(variable) = &source.variable {
                bound.add(Mode::Input, variable, item);
            }
            bound
        };
        if let Some(condition) = &source.condition {
            let mut kept = Vec::with_capacity(items.len());
            for item in items {
                if self
                    .evaluate_boolean(run, group, condition, Some(item), &with_item(item))
                    .await?
                {
                    kept.push(item);
                }
            }
            items = kept;
        }
        if let Some(check) = &source.check {
            for &item in &items {
                if !self
                    .evaluate_boolean(run, group, check, Some(item), &with_item(item))
                    .await?
                {
                    return Err(Error::in_rule(rule, format!("check `{check}` failed")));
                }
            }
        }
        if let Some(log) = &source.log_message {
            let mut messages = Vec::new();
            for &item in &items {
                let values = self
                    .evaluate(run, group, log, Some(item), &with_item(item))
                    .await?;
                messages.extend(values.into_iter().filter_map(|v| run.tree.text(v)));
            }
            if !messages.is_empty() {
                run.log.push(messages.join(","));
            }
        }
        match source.list_mode.as_deref() {
            _ if items.is_empty() => {}
            Some("first") => items.truncate(1),
            Some("not_first") => {
                items.remove(0);
            }
            Some("last") => items = vec![*items.last().unwrap()],
            Some("not_last") => {
                items.pop();
            }
            Some("only_one") if items.len() > 1 => {
                return Err(Error::in_rule(
                    rule,
                    format!("`{}` has more than one item", source.context),
                ));
            }
            _ => {}
        }
        Ok(items.into_iter().map(with_item).collect())
    }

    // ---- Targets ----------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn process_target(
        &self,
        run: &mut Run,
        group: &GroupRef,
        rule: &Rule,
        target: &Target,
        vars: &mut Vars,
        shared: &mut Vars,
        source_variable: Option<&str>,
        at_root: bool,
    ) -> Result<()> {
        let types = self.types();
        let elements = element_path(
            target.element.as_deref(),
            &target.extension,
            TARGET_ELEMENT_EXTENSION,
        );
        let element = elements.last().cloned();
        let mut destination = match &target.context {
            Some(context) => Some(vars.get(Mode::Output, context).ok_or_else(|| {
                Error::in_rule(rule, format!("unknown target variable `{context}`"))
            })?),
            None => None,
        };
        let sharing = target.list_mode.iter().any(|m| m == "share");
        let mut value = match (&target.transform, sharing, &target.list_rule_id) {
            (None, true, Some(id)) => shared.get(Mode::Shared, id),
            _ => None,
        };
        if let Some(dest) = destination
            && value.is_none()
        {
            let mut dest = dest;
            for intermediate in elements.iter().take(elements.len().saturating_sub(1)) {
                dest = run
                    .tree
                    .make_property(dest, intermediate, &types)
                    .map_err(|e| Error::in_rule(rule, e))?;
            }
            destination = Some(dest);
        }
        if target.transform.is_some() {
            let produced = if target.transform.as_deref() == Some("evaluate") {
                let values = self
                    .evaluate_transform(run, group, rule, target, vars)
                    .await?;
                if values.len() > 1 {
                    self.check_several(
                        run,
                        rule,
                        target,
                        destination,
                        element.as_deref(),
                        values.len(),
                    )?;
                }
                values
            } else {
                self.run_transform(
                    run,
                    group,
                    rule,
                    target,
                    vars,
                    destination,
                    element.as_deref(),
                    source_variable,
                    at_root,
                )
                .await?
                .into_iter()
                .collect()
            };
            if let Some(dest) = destination {
                for &item in &produced {
                    match &element {
                        Some(element) => {
                            run.tree
                                .set_property(dest, element, item, &types)
                                .map_err(|e| Error::in_rule(rule, e))?;
                            order_target(run, rule, target, dest, element, item)?;
                        }
                        None => run.tree.assign(dest, item),
                    }
                }
            }
            value = produced.last().copied();
        } else if let (Some(dest), None) = (destination, value) {
            value = Some(match &element {
                Some(element) => {
                    let created = run
                        .tree
                        .make_property(dest, element, &types)
                        .map_err(|e| Error::in_rule(rule, e))?;
                    if let (true, Some(id)) = (sharing, &target.list_rule_id) {
                        // Siblings find it in the enclosing scope, nested rules in this one.
                        shared.add(Mode::Shared, id, created);
                        vars.add(Mode::Shared, id, created);
                    }
                    order_target(run, rule, target, dest, element, created)?;
                    created
                }
                None => dest,
            });
        }
        if let (Some(variable), Some(value)) = (&target.variable, value) {
            vars.add(Mode::Output, variable, value);
        }
        Ok(())
    }

    /// `evaluate(context, expression)` or `(expression)`: every value the expression gives.
    async fn evaluate_transform(
        &self,
        run: &mut Run,
        group: &GroupRef,
        rule: &Rule,
        target: &Target,
        vars: &Vars,
    ) -> Result<Vec<NodeId>> {
        let parameters = &target.parameter;
        let expression = match parameters.last().map(|p| &p.value) {
            Some(ParameterValue::String(expression)) => expression,
            _ => return Err(Error::in_rule(rule, "evaluate needs an expression")),
        };
        let focus = match parameters.as_slice() {
            [context, _] => Some(self.parameter_node(run, group, context, vars).await?),
            _ => None,
        };
        self.evaluate(run, group, expression, focus, vars).await
    }

    /// Several values from `evaluate` make one target each, which needs a repeating element
    /// (transforms.md, "evaluate"); a variable cannot name them all.
    fn check_several(
        &self,
        run: &Run,
        rule: &Rule,
        target: &Target,
        destination: Option<NodeId>,
        element: Option<&str>,
        count: usize,
    ) -> Result<()> {
        let fail = |message: String| Err(Error::in_rule(rule, message));
        let (Some(dest), Some(element)) = (destination, element) else {
            return fail(format!(
                "evaluate gave {count} values where one was expected"
            ));
        };
        let owner = run.tree.ty(dest).display();
        match self.types().lookup(run.tree.ty(dest), element) {
            Lookup::Found(found) if found.element.array => {}
            Lookup::Found(_) => {
                return fail(format!(
                    "evaluate gave {count} values, but `{owner}.{element}` does not repeat"
                ));
            }
            _ => {
                return fail(format!(
                    "evaluate gave {count} values, and whether `{owner}.{element}` repeats is not known"
                ));
            }
        }
        if let Some(variable) = &target.variable {
            return fail(format!(
                "evaluate gave {count} values; variable `{variable}` can only name one"
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_transform(
        &self,
        run: &mut Run,
        group: &GroupRef,
        rule: &Rule,
        target: &Target,
        vars: &Vars,
        destination: Option<NodeId>,
        element: Option<&str>,
        source_variable: Option<&str>,
        _at_root: bool,
    ) -> Result<Option<NodeId>> {
        let transform = target.transform.as_deref().unwrap_or("copy");
        let parameters = &target.parameter;
        let fail = |message: String| Error::in_rule(rule, message);
        let types = self.types();
        match transform {
            "create" => {
                let ty = match parameters.first() {
                    Some(parameter) => {
                        let name = self.parameter_text(run, group, parameter, vars).await?;
                        let context = destination
                            .and_then(|d| run.tree.ty(d).structure().map(|s| s.url.clone()))
                            .unwrap_or_else(|| CORE.to_owned());
                        let ty = types.structure(&self.type_name_url(&group.map, &name, &context));
                        (ty, Some(name))
                    }
                    None => self
                        .implied_type(run, group, destination, element, source_variable, vars)
                        .map_err(fail)?,
                };
                let created = run.tree.create(ty.0, ty.1);
                // A resource created outside any target element is one the map produces.
                if destination.is_none() && run.tree.ty(created).is_resource() {
                    run.produced.push(created);
                }
                Ok(Some(created))
            }
            "copy" => {
                let parameter = parameters
                    .first()
                    .ok_or_else(|| fail("copy needs a value".into()))?;
                let node = self.parameter_node(run, group, parameter, vars).await?;
                Ok(Some(run.tree.deep_copy(node)))
            }
            "truncate" => {
                let text = self.argument_text(run, group, parameters, 0, vars).await?;
                let length = self.argument_text(run, group, parameters, 1, vars).await?;
                let length: usize = length
                    .trim()
                    .parse()
                    .map_err(|_| fail(format!("truncate needs a length, not `{length}`")))?;
                let truncated = text.chars().take(length).collect();
                Ok(Some(self.string(run, truncated, "string")))
            }
            "cast" => {
                let source = self
                    .parameter_node(
                        run,
                        group,
                        parameters
                            .first()
                            .ok_or_else(|| fail("cast needs a value".into()))?,
                        vars,
                    )
                    .await?;
                let type_name = if parameters.len() > 1 {
                    transforms::cast_type(
                        &self.argument_text(run, group, parameters, 1, vars).await?,
                    )
                } else {
                    // Without a type, the value takes the type of the element it is written to.
                    self.implied_type(run, group, destination, element, None, vars)
                        .ok()
                        .and_then(|(_, name)| name)
                        .ok_or_else(|| fail("cast needs a type: cast(value, 'type')".into()))?
                };
                if !run.tree.is_primitive(source) {
                    // A complex value casts only to its own type.
                    return match run.tree.fhir_type(source) {
                        Some(own) if own == type_name => Ok(Some(run.tree.deep_copy(source))),
                        own => Err(fail(format!(
                            "cannot cast a {} to {type_name}",
                            own.as_deref().unwrap_or("complex value")
                        ))),
                    };
                }
                let text = run.tree.text(source).unwrap_or_default();
                let value = transforms::cast(&text, &type_name).map_err(fail)?;
                let ty = types.structure(&type_url(&type_name, CORE));
                Ok(Some(run.tree.primitive(value, ty, Some(type_name))))
            }
            "append" => {
                let mut text = String::new();
                for index in 0..parameters.len() {
                    text.push_str(
                        &self
                            .argument_text(run, group, parameters, index, vars)
                            .await?,
                    );
                }
                Ok(Some(self.string(run, text, "string")))
            }
            "translate" => self.translate(run, group, rule, parameters, vars).await,
            "reference" | "pointer" => {
                let parameter = parameters
                    .first()
                    .ok_or_else(|| fail(format!("{transform} needs a resource")))?;
                let resource = self.parameter_node(run, group, parameter, vars).await?;
                let resource_type = run
                    .tree
                    .fhir_type(resource)
                    .filter(|_| {
                        run.tree.ty(resource).is_resource() || !run.tree.is_primitive(resource)
                    })
                    .ok_or_else(|| fail(format!("{transform} needs a resource")))?;
                let id = match run
                    .tree
                    .children(resource, "id")
                    .first()
                    .and_then(|&id| run.tree.text(id))
                {
                    Some(id) => id,
                    None => {
                        let id = self.ids.uuid();
                        run.tree
                            .set_text(resource, "id", &id, &types)
                            .map_err(fail)?;
                        id
                    }
                };
                Ok(Some(if transform == "reference" {
                    self.string(run, format!("{resource_type}/{id}"), "string")
                } else {
                    self.string(run, format!("urn:uuid:{id}"), "uri")
                }))
            }
            "uuid" => Ok(Some(self.string(run, self.ids.uuid(), "id"))),
            "toDateTime" | "toDate" | "toTime" => {
                let text = self.argument_text(run, group, parameters, 0, vars).await?;
                let format = match parameters.get(1) {
                    Some(_) => Some(self.argument_text(run, group, parameters, 1, vars).await?),
                    None => None,
                };
                let (value, type_name) = match transform {
                    "toDateTime" => (datetime::to_date_time(&text, format.as_deref()), "dateTime"),
                    "toDate" => (datetime::to_date(&text, format.as_deref()), "date"),
                    _ => (datetime::to_time(&text, format.as_deref()), "time"),
                };
                Ok(Some(self.string(run, value.map_err(fail)?, type_name)))
            }
            "unixToDateTime" | "unixToDate" | "unixToTime" => {
                let text = self.argument_text(run, group, parameters, 0, vars).await?;
                let millis: i64 = text.trim().parse().map_err(|_| {
                    fail(format!("`{text}` is not a Unix timestamp in milliseconds"))
                })?;
                let timezone = match parameters.get(1) {
                    Some(_) => Some(self.argument_text(run, group, parameters, 1, vars).await?),
                    None => None,
                };
                let (value, type_name) = match transform {
                    "unixToDateTime" => (
                        datetime::unix_to_date_time(millis, timezone.as_deref()),
                        "dateTime",
                    ),
                    "unixToDate" => (datetime::unix_to_date(millis, timezone.as_deref()), "date"),
                    _ => (datetime::unix_to_time(millis, timezone.as_deref()), "time"),
                };
                Ok(Some(self.string(run, value.map_err(fail)?, type_name)))
            }
            "escape" => {
                let text = self.argument_text(run, group, parameters, 0, vars).await?;
                let (from, to) = match parameters.len() {
                    3 => (
                        self.argument_text(run, group, parameters, 1, vars).await?,
                        self.argument_text(run, group, parameters, 2, vars).await?,
                    ),
                    _ => (
                        "none".to_owned(),
                        self.argument_text(run, group, parameters, 1, vars).await?,
                    ),
                };
                let escaped = datetime::escape(&text, &from, &to).map_err(fail)?;
                Ok(Some(self.string(run, escaped, "string")))
            }
            "cc" | "c" | "id" | "cp" | "qty" => {
                let mut arguments = Vec::with_capacity(parameters.len());
                for index in 0..parameters.len() {
                    arguments.push(
                        self.argument_text(run, group, parameters, index, vars)
                            .await?,
                    );
                }
                let (json, type_name) = match transform {
                    "cc" => (transforms::codeable_concept(&arguments), "CodeableConcept"),
                    "c" => match arguments.as_slice() {
                        [system, code, rest @ ..] => {
                            (transforms::coding(system, code, rest.first()), "Coding")
                        }
                        _ => return Err(fail("c needs a system and a code".into())),
                    },
                    "id" => (transforms::identifier(&arguments), "Identifier"),
                    "cp" => (transforms::contact_point(&arguments), "ContactPoint"),
                    _ => (transforms::quantity(&arguments).map_err(fail)?, "Quantity"),
                };
                let context = destination
                    .and_then(|d| run.tree.ty(d).structure().map(|s| s.url.clone()))
                    .unwrap_or_else(|| CORE.to_owned());
                let ty = types.structure(&type_url(type_name, &context));
                let node = run.tree.load(&json, ty, &types);
                Ok(Some(node))
            }
            other => Err(fail(format!("the {other} transform is not supported"))),
        }
    }

    /// The type `create()` makes without a type argument: the element's only type, or the
    /// source's type.
    fn implied_type(
        &self,
        run: &Run,
        group: &GroupRef,
        destination: Option<NodeId>,
        element: Option<&str>,
        source_variable: Option<&str>,
        vars: &Vars,
    ) -> std::result::Result<(Type, Option<String>), String> {
        let types = self.types();
        if let (Some(dest), Some(element)) = (destination, element)
            && let Lookup::Found(found) = types.lookup(run.tree.ty(dest), element)
            && let [single] = found.element.types.as_slice()
            && single != "Resource"
            && single != "*"
        {
            return Ok((types.element_type(&found, None), Some(single.clone())));
        }
        if let Some(source) = source_variable.and_then(|s| vars.get(Mode::Input, s)) {
            // The type the default group for the source's type produces, as the reference engine
            // does: a contained R4 Medication becomes the R5 Medication its group maps to.
            if let Some(source_type) = run.tree.fhir_type(source)
                && let Some(produced) = self.default_target_type(group, &source_type)
            {
                return Ok(produced);
            }
            return Ok((run.tree.ty(source).clone(), run.tree.fhir_type(source)));
        }
        Err("cannot tell what type to create: name it, as in create('Type')".into())
    }

    /// The target type of the `<<type+>>` group whose source is `source_type`, in the map or the
    /// maps it imports.
    fn default_target_type(&self, group: &GroupRef, source_type: &str) -> Option<TypedTarget> {
        if let Some(found) = self.default_target_in(&group.map, source_type) {
            return Some(found);
        }
        let key = (imports_key(&group.map), source_type.to_owned());
        if let Some(found) = self.cache().default_target.get(&key) {
            return found.clone();
        }
        let found = self.find_default_target_type(group, source_type);
        self.cache().default_target.insert(key, found.clone());
        found
    }

    /// The target type of a `<<type+>>` group of `map` itself whose source is `source_type`.
    fn default_target_in(&self, map: &StructureMap, source_type: &str) -> Option<TypedTarget> {
        map.group.iter().find_map(|candidate| {
            if candidate.type_mode.as_deref() != Some("type-and-types") {
                return None;
            }
            let [source, target] = candidate.input.as_slice() else {
                return None;
            };
            if source.mode != "source"
                || target.mode != "target"
                || !self.stated_type_is(map, source.type_name.as_deref(), source_type)
            {
                return None;
            }
            let stated = target.type_name.as_deref()?;
            let ty = self
                .types()
                .structure(&self.type_name_url(map, stated, CORE));
            let name = ty.fhir_type().unwrap_or_else(|| stated.to_owned());
            Some((ty, Some(name)))
        })
    }

    fn find_default_target_type(&self, group: &GroupRef, source_type: &str) -> Option<TypedTarget> {
        group.map.import.iter().find_map(|import| {
            self.maps
                .resolve(import)
                .iter()
                .filter(|imported| imported.url != group.map.url)
                .find_map(|imported| self.default_target_in(imported, source_type))
        })
    }

    /// `translate(value, map, output)`: a code, Coding or CodeableConcept translated with a
    /// ConceptMap contained in the map (`'#id'`) or found by the ConceptMap resolver.
    async fn translate(
        &self,
        run: &mut Run,
        group: &GroupRef,
        rule: &Rule,
        parameters: &[Parameter],
        vars: &Vars,
    ) -> Result<Option<NodeId>> {
        let fail = |message: String| Error::in_rule(rule, message);
        let source = self
            .parameter_node(
                run,
                group,
                parameters
                    .first()
                    .ok_or_else(|| fail("translate needs a value".into()))?,
                vars,
            )
            .await?;
        let map_url = self.argument_text(run, group, parameters, 1, vars).await?;
        let output = self.argument_text(run, group, parameters, 2, vars).await?;
        let concept_map = match map_url.strip_prefix('#') {
            Some(id) => Some(
                group
                    .map
                    .contained
                    .iter()
                    .find(|r| r["resourceType"] == "ConceptMap" && r["id"] == id)
                    .cloned()
                    .map(Arc::new)
                    .ok_or_else(|| fail(format!("no contained ConceptMap `{id}`")))?,
            ),
            None => self.concept_maps.resolve(&map_url),
        };
        if concept_map.is_none() && self.translator.is_none() {
            return Err(fail(format!("ConceptMap `{map_url}` not found")));
        }
        let codes = self.codes_of(run, source);
        let mut translation = None;
        for (system, code) in &codes {
            let found = match (&concept_map, self.translator) {
                (Some(concept_map), _) => translate(concept_map, system.as_deref(), code),
                (None, Some(translator)) => {
                    translator
                        .translate(&map_url, system.as_deref(), code)
                        .await
                }
                (None, None) => unreachable!("checked above"),
            }
            .map_err(|e| fail(format!("{e} in `{map_url}`")))?;
            if let Some(found) = found {
                translation = Some(found);
                break;
            }
        }
        let Some(translation) = translation else {
            let shown: Vec<&str> = codes.iter().map(|(_, c)| c.as_str()).collect();
            let rule_name = rule.name.as_deref().unwrap_or("?");
            run.warnings.push(format!(
                "rule `{rule_name}`: `{}` has no translation in `{map_url}`; left out",
                shown.join(", ")
            ));
            return Ok(None);
        };

        let types = self.types();
        let mut coding = serde_json::Map::new();
        if let Some(system) = &translation.system {
            coding.insert("system".into(), system.clone().into());
        }
        if let Some(code) = &translation.code {
            coding.insert("code".into(), code.clone().into());
        }
        if let Some(display) = &translation.display {
            coding.insert("display".into(), display.clone().into());
        }
        let text = |value: &Option<String>| value.clone().map_or(Value::Null, Value::String);
        let (json, type_name) = match output.as_str() {
            "code" => (text(&translation.code), "code"),
            "system" => (text(&translation.system), "uri"),
            "display" => (text(&translation.display), "string"),
            "Coding" => (Value::Object(coding), "Coding"),
            "CodeableConcept" => (
                json!({ "coding": [Value::Object(coding)] }),
                "CodeableConcept",
            ),
            other => return Err(fail(format!("unknown translate output `{other}`"))),
        };
        let ty = types.structure(&type_url(type_name, CORE));
        Ok(Some(if json.is_object() {
            run.tree.load(&json, ty, &types)
        } else {
            run.tree.primitive(json, ty, Some(type_name.to_owned()))
        }))
    }

    /// The (system, code) pairs a value holds: a code, a Coding, or a CodeableConcept's codings.
    fn codes_of(&self, run: &Run, node: NodeId) -> Vec<(Option<String>, String)> {
        let text = |id: NodeId, name: &str| {
            run.tree
                .children(id, name)
                .first()
                .and_then(|&child| run.tree.text(child))
        };
        if run.tree.is_primitive(node) {
            return run
                .tree
                .text(node)
                .map(|code| (None, code))
                .into_iter()
                .collect();
        }
        let codings = run.tree.children(node, "coding");
        let codings = if codings.is_empty() {
            vec![node]
        } else {
            codings
        };
        codings
            .into_iter()
            .filter_map(|coding| Some((text(coding, "system"), text(coding, "code")?)))
            .collect()
    }

    // ---- Parameters -------------------------------------------------------------------------

    /// A parameter as a node: a variable (or constant), or a new primitive for a literal.
    async fn parameter_node(
        &self,
        run: &mut Run,
        group: &GroupRef,
        parameter: &Parameter,
        vars: &Vars,
    ) -> Result<NodeId> {
        let types = self.types();
        let (value, type_name) = match &parameter.value {
            ParameterValue::Id(name) => return self.variable(run, group, name, vars).await,
            ParameterValue::String(v) => (Value::String(v.clone()), "string"),
            ParameterValue::Boolean(v) => (Value::Bool(*v), "boolean"),
            ParameterValue::Integer(v) => (Value::from(*v), "integer"),
            ParameterValue::Decimal(v) => (Value::Number(v.clone()), "decimal"),
            ParameterValue::Date(v) => (Value::String(v.clone()), "date"),
            ParameterValue::Time(v) => (Value::String(v.clone()), "time"),
            ParameterValue::DateTime(v) => (Value::String(v.clone()), "dateTime"),
        };
        let ty = types.structure(&type_url(type_name, CORE));
        Ok(run.tree.primitive(value, ty, Some(type_name.to_owned())))
    }

    /// A variable, or failing that a constant of the map.
    async fn variable(
        &self,
        run: &mut Run,
        group: &GroupRef,
        name: &str,
        vars: &Vars,
    ) -> Result<NodeId> {
        if let Some(node) = vars.value(name) {
            return Ok(node);
        }
        // A dotted id such as `src.code` is a path from a variable.
        if let Some((variable, path)) = name.split_once('.')
            && let Some(mut node) = vars.value(variable)
        {
            for segment in path.split('.') {
                node = *run
                    .tree
                    .children(node, segment)
                    .first()
                    .ok_or_else(|| Error::new(format!("`{name}` has no value")))?;
            }
            return Ok(node);
        }
        let values = self.constant(run, group, name).await?;
        match values.as_slice() {
            [single] => Ok(*single),
            [] => Err(Error::new(format!("unknown variable `{name}`"))),
            _ => Err(Error::new(format!("constant `{name}` has several values"))),
        }
    }

    async fn parameter_text(
        &self,
        run: &mut Run,
        group: &GroupRef,
        parameter: &Parameter,
        vars: &Vars,
    ) -> Result<String> {
        if let ParameterValue::String(text) = &parameter.value {
            return Ok(text.clone());
        }
        let node = self.parameter_node(run, group, parameter, vars).await?;
        Ok(run.tree.text(node).unwrap_or_default())
    }

    async fn argument_text(
        &self,
        run: &mut Run,
        group: &GroupRef,
        parameters: &[Parameter],
        index: usize,
        vars: &Vars,
    ) -> Result<String> {
        let parameter = parameters
            .get(index)
            .ok_or_else(|| Error::new(format!("missing argument {}", index + 1)))?;
        self.parameter_text(run, group, parameter, vars).await
    }

    fn string(&self, run: &mut Run, text: String, type_name: &str) -> NodeId {
        let ty = self.types().structure(&type_url(type_name, CORE));
        run.tree
            .primitive(Value::String(text), ty, Some(type_name.to_owned()))
    }

    // ---- FHIRPath ---------------------------------------------------------------------------

    /// The value of the map's `let name = …;`, evaluated once.
    fn constant<'r>(
        &'r self,
        run: &'r mut Run,
        group: &'r GroupRef,
        name: &'r str,
    ) -> BoxFuture<'r, Result<Vec<NodeId>>> {
        Box::pin(async move {
            let key = (group.map.url.clone().unwrap_or_default(), name.to_owned());
            if let Some(values) = run.constants.get(&key) {
                return Ok(values.clone());
            }
            let Some(constant) = group.map.constants.iter().find(|c| c.name == name) else {
                return Ok(Vec::new());
            };
            if !run.evaluating.insert(key.clone()) {
                return Err(Error::new(format!("constant `{name}` refers to itself")));
            }
            let values = self
                .evaluate(run, group, &constant.value, None, &Vars::default())
                .await;
            run.evaluating.remove(&key);
            let values = values?;
            run.constants.insert(key, values.clone());
            Ok(values)
        })
    }

    fn evaluate<'r>(
        &'r self,
        run: &'r mut Run,
        group: &'r GroupRef,
        expression: &'r str,
        focus: Option<NodeId>,
        vars: &'r Vars,
    ) -> BoxFuture<'r, Result<Vec<NodeId>>> {
        Box::pin(async move {
            // Only the variables the expression names: serializing every variable in scope for
            // every expression would copy the whole input each time.
            let referenced = path_start_names(expression);
            let mut names: HashSet<String> = vars
                .names()
                .filter(|name| referenced.contains(*name))
                .map(str::to_owned)
                .collect();
            let mut variables = Vec::new();
            for name in &names {
                if let Some(node) = vars.value(name) {
                    variables.push((name.clone(), self.item(run, node)));
                }
            }
            for constant in &group.map.constants {
                if names.contains(&constant.name) || expression == constant.value {
                    continue;
                }
                if referenced.contains(&constant.name) {
                    let values = self.constant(run, group, &constant.name).await?;
                    if let [value] = values.as_slice() {
                        variables.push((constant.name.clone(), self.item(run, *value)));
                        names.insert(constant.name.clone());
                    }
                }
            }
            let bound = bind_variables(expression, &names);
            let focus = focus.map(|node| self.item(run, node));
            let results = self
                .expressions
                .evaluate(&bound, focus.as_ref(), &variables)
                .await
                .map_err(|e| Error::new(format!("`{expression}`: {e}")))?;
            let types = self.types();
            Ok(results
                .into_iter()
                .map(|item| {
                    let ty = item
                        .type_name
                        .as_deref()
                        .map(|t| types.structure(&type_url(t, CORE)))
                        .unwrap_or(Type::Unknown);
                    if item.json.is_object() {
                        run.tree.load(&item.json, ty, &types)
                    } else {
                        run.tree.primitive(item.json, ty, item.type_name)
                    }
                })
                .collect())
        })
    }

    async fn evaluate_boolean(
        &self,
        run: &mut Run,
        group: &GroupRef,
        expression: &str,
        focus: Option<NodeId>,
        vars: &Vars,
    ) -> Result<bool> {
        let values = self.evaluate(run, group, expression, focus, vars).await?;
        Ok(match values.as_slice() {
            [] => false,
            [single] => run.tree.value(*single) != Some(&Value::Bool(false)),
            _ => true,
        })
    }

    /// A node for FHIRPath. A backbone element is named by its path (`QuestionnaireResponse.item`),
    /// as FHIRPath models it, so its elements keep their FHIR types.
    fn item(&self, run: &Run, node: NodeId) -> Item {
        let type_name = match run.tree.ty(node) {
            backbone @ Type::Node { path, .. } if !path.is_empty() => Some(backbone.display()),
            _ => run.tree.fhir_type(node),
        };
        Item {
            json: run.tree.to_json(node),
            type_name,
        }
    }
}

/// The element path of a source or target: the element and the segments in the element
/// extension.
fn element_path(
    element: Option<&str>,
    extensions: &[fml_model::model::Extension],
    url: &str,
) -> Vec<String> {
    element
        .map(str::to_owned)
        .into_iter()
        .chain(
            extensions
                .iter()
                .filter(|e| e.url == url)
                .filter_map(|e| e.value_string.clone()),
        )
        .collect()
}

/// Target list modes (the `StructureMapTargetListMode` codes; the IG leaves them to be
/// documented): a `first` rule's values go before the others in a repeating element and a `last`
/// rule's after them, one rule each per element; `single` allows one value per element.
fn order_target(
    run: &mut Run,
    rule: &Rule,
    target: &Target,
    dest: NodeId,
    element: &str,
    value: NodeId,
) -> Result<()> {
    let identity = std::ptr::from_ref(target) as usize;
    let mode = target
        .list_mode
        .iter()
        .find(|mode| matches!(mode.as_str(), "first" | "last" | "single"))
        .map(String::as_str);
    if mode == Some("single") && !run.single.insert((dest, element.to_owned(), identity)) {
        return Err(Error::in_rule(
            rule,
            format!("target `{element}` is `single`, but the rule makes more than one"),
        ));
    }
    let key = (dest, element.to_owned());
    let order = match (mode, run.order.contains_key(&key)) {
        (Some("first" | "last"), _) | (_, true) => run.order.entry(key).or_default(),
        _ => return Ok(()),
    };
    match mode {
        Some("first") => {
            if order.first.is_some_and(|first| first != identity) {
                return Err(Error::in_rule(
                    rule,
                    format!("two rules put their values `first` in `{element}`"),
                ));
            }
            order.first = Some(identity);
            let at = order.first_count;
            order.first_count += 1;
            run.tree.move_value(dest, element, value, Some(at));
        }
        Some("last") => {
            if order.last.is_some_and(|last| last != identity) {
                return Err(Error::in_rule(
                    rule,
                    format!("two rules put their values `last` in `{element}`"),
                ));
            }
            order.last = Some(identity);
            order.last_values.push(value);
        }
        _ => {}
    }
    let last_values = order.last_values.clone();
    for last in last_values {
        run.tree.move_value(dest, element, last, None);
    }
    Ok(())
}

/// A source's `min..max`: the element must have that many values (fml.md, "Source Content").
fn check_cardinality(rule: &Rule, source: &Source, count: usize) -> Result<()> {
    let below = source
        .min
        .is_some_and(|min| i64::try_from(count).is_ok_and(|count| count < min));
    let above = source
        .max
        .as_deref()
        .and_then(|max| max.parse::<usize>().ok())
        .is_some_and(|max| count > max);
    if below || above {
        let path = std::iter::once(source.context.as_str())
            .chain(source.element.as_deref())
            .collect::<Vec<_>>()
            .join(".");
        return Err(Error::in_rule(
            rule,
            format!(
                "`{path}` has {count} values, outside {}..{}",
                source.min.unwrap_or(0),
                source.max.as_deref().unwrap_or("*")
            ),
        ));
    }
    Ok(())
}

/// The key of a map's imports in the group cache: its URL and what it imports.
fn imports_key(map: &StructureMap) -> String {
    let mut key = map.url.clone().unwrap_or_default();
    for import in &map.import {
        key.push('\n');
        key.push_str(import);
    }
    key
}

/// A dependent group argument's variable name.
fn parameter_name(parameter: &Parameter) -> String {
    match &parameter.value {
        ParameterValue::Id(name) | ParameterValue::String(name) => name.clone(),
        other => format!("{other:?}"),
    }
}

/// The variables of a rule that copies one value to one target with nothing else to do.
fn simple_copy(rule: &Rule) -> Option<(&str, &str)> {
    let ([source], [target]) = (rule.source.as_slice(), rule.target.as_slice()) else {
        return None;
    };
    (target.transform.as_deref() == Some("create") && target.parameter.is_empty()).then_some(())?;
    Some((source.variable.as_deref()?, target.variable.as_deref()?))
}
