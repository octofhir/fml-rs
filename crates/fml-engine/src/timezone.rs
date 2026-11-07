//! Time zone names, for the `z` format code ("Time zone literal (name or id)") and the timezone of
//! the `unixTo*` transforms:
//!
//! - IANA ids (`America/Los_Angeles`), from the time zone database bundled with `jiff`;
//! - Windows names (`Pacific Standard Time`), through the CLDR mapping to IANA ids;
//! - `UTC`, `GMT`, and abbreviations that name a single offset (`PST`, `CEST`, `JST`).
//!
//! Abbreviations that several zones use (`CST`: US Central, China, Cuba; `IST`; `BST`) are
//! refused rather than guessed. A named zone gives the offset in force at the date and time it is
//! applied to, daylight saving included.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::{AmbiguousOffset, TimeZone};

use crate::windows_zones::WINDOWS_ZONES;

/// A time zone the source or a parameter names.
#[derive(Debug, Clone)]
pub(crate) enum Zone {
    /// `UTC`, `GMT`: written `Z`.
    Utc,
    /// An abbreviation with one offset, in seconds east of UTC.
    Fixed(i32),
    Named(TimeZone),
}

/// Abbreviations that name one offset, in minutes east of UTC.
const ABBREVIATIONS: &[(&str, i32)] = &[
    ("HST", -600),
    ("AKST", -540),
    ("AKDT", -480),
    ("PST", -480),
    ("PDT", -420),
    ("MST", -420),
    ("MDT", -360),
    ("EST", -300),
    ("EDT", -240),
    ("NST", -210),
    ("NDT", -150),
    ("WET", 0),
    ("WEST", 60),
    ("CET", 60),
    ("CEST", 120),
    ("EET", 120),
    ("EEST", 180),
    ("MSK", 180),
    ("HKT", 480),
    ("SGT", 480),
    ("AWST", 480),
    ("JST", 540),
    ("KST", 540),
    ("ACST", 570),
    ("ACDT", 630),
    ("AEST", 600),
    ("AEDT", 660),
    ("NZST", 720),
    ("NZDT", 780),
];

/// Abbreviations used by more than one zone.
const AMBIGUOUS: &[&str] = &[
    "CST", "CDT", "IST", "BST", "AST", "ADT", "SST", "GST", "ECT",
];

/// The zone `name` names.
pub(crate) fn zone(name: &str) -> Result<Zone, String> {
    if matches!(name, "UTC" | "GMT" | "Z") {
        return Ok(Zone::Utc);
    }
    if let Some((_, minutes)) = ABBREVIATIONS.iter().find(|(a, _)| *a == name) {
        return Ok(Zone::Fixed(minutes * 60));
    }
    if AMBIGUOUS.contains(&name) {
        return Err(format!(
            "`{name}` names more than one time zone; use an offset such as -06:00 or an id such as America/Chicago"
        ));
    }
    let id = WINDOWS_ZONES
        .iter()
        .find(|(windows, _)| *windows == name)
        .map_or(name, |(_, iana)| *iana);
    TimeZone::get(id)
        .map(Zone::Named)
        .map_err(|_| format!("`{name}` is not a time zone this engine knows"))
}

/// The zone name at the start of `text`, and its length: a Windows name (they contain spaces),
/// or a run of the characters of ids and abbreviations.
pub(crate) fn read_zone(text: &str) -> Result<(Zone, usize), String> {
    if let Some((windows, _)) = WINDOWS_ZONES.iter().find(|(w, _)| text.starts_with(w)) {
        return Ok((zone(windows)?, windows.len()));
    }
    let length = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '+' | '-')))
        .unwrap_or(text.len());
    if length == 0 {
        return Err("a time zone name is missing".to_owned());
    }
    Ok((zone(&text[..length])?, length))
}

impl Zone {
    /// The offset in seconds at a local date and time. In a daylight saving gap or overlap, the
    /// offset in force before the transition: the written clock time is kept as it is.
    pub(crate) fn offset_at_local(
        &self,
        year: i64,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Result<i32, String> {
        match self {
            Zone::Utc => Ok(0),
            Zone::Fixed(seconds) => Ok(*seconds),
            Zone::Named(tz) => {
                let out_of_range = || format!("{year:04}-{month:02}-{day:02} is out of range");
                let local = DateTime::new(
                    i16::try_from(year).map_err(|_| out_of_range())?,
                    month as i8,
                    day as i8,
                    hour as i8,
                    minute as i8,
                    second as i8,
                    0,
                )
                .map_err(|e| e.to_string())?;
                let offset = match tz.to_ambiguous_timestamp(local).offset() {
                    AmbiguousOffset::Unambiguous { offset } => offset,
                    AmbiguousOffset::Gap { before, .. } | AmbiguousOffset::Fold { before, .. } => {
                        before
                    }
                };
                Ok(offset.seconds())
            }
        }
    }

    /// The offset in seconds at an instant, in seconds since the Unix epoch.
    pub(crate) fn offset_at(&self, seconds: i64) -> Result<i32, String> {
        match self {
            Zone::Utc => Ok(0),
            Zone::Fixed(offset) => Ok(*offset),
            Zone::Named(tz) => {
                let instant = Timestamp::from_second(seconds).map_err(|e| e.to_string())?;
                Ok(tz.to_offset(instant).seconds())
            }
        }
    }

    /// The offset as FHIR writes it: `Z` for UTC, `+hh:mm` otherwise.
    pub(crate) fn text(&self, offset: i32) -> String {
        if matches!(self, Zone::Utc) {
            return "Z".to_owned();
        }
        let sign = if offset < 0 { '-' } else { '+' };
        let minutes = offset.unsigned_abs() / 60;
        format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
    }
}
