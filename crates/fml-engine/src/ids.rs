//! New ids for `uuid()` and for resources `reference()` gives an id: random by default, or
//! fixed by the caller so runs can be compared.

use std::sync::atomic::{AtomicU64, Ordering};

/// Makes the UUIDs a transform needs.
pub trait IdGenerator {
    /// A new UUID, lowercase.
    fn uuid(&self) -> String;
}

/// Random (version 4) UUIDs.
#[derive(Debug, Default, Clone, Copy)]
pub struct RandomIds;

impl IdGenerator for RandomIds {
    fn uuid(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// UUIDs counting up from zero (`00000000-0000-0000-0000-000000000001`, …), for tests and
/// repeatable runs.
#[derive(Debug, Default)]
pub struct SequentialIds {
    next: AtomicU64,
}

impl SequentialIds {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IdGenerator for SequentialIds {
    fn uuid(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        uuid::Uuid::from_u128(u128::from(n)).to_string()
    }
}
