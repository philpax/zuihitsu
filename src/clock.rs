//! The clock seam. `now()` reads an injected clock so tests can drive time explicitly — recency
//! decay, calendar windows, scheduled wake-ups — without real time passing (spec §Testability).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ids::Timestamp;

/// A source of "now". Shared immutably across threads, so it is `Send + Sync`.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The production clock, backed by the system wall clock (UTC).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Timestamp::from_millis(millis)
    }
}

/// A test clock advanced explicitly. Cloning shares the same underlying time, so a clone handed to
/// the system under test and a clone retained by the test observe the same advances.
#[derive(Clone, Debug)]
pub struct ManualClock {
    millis: Arc<AtomicI64>,
}

impl ManualClock {
    pub fn new(start: Timestamp) -> ManualClock {
        ManualClock {
            millis: Arc::new(AtomicI64::new(start.as_millis())),
        }
    }

    /// Set the clock to an absolute time.
    pub fn set(&self, now: Timestamp) {
        self.millis.store(now.as_millis(), Ordering::SeqCst);
    }

    /// Move the clock forward by a number of milliseconds.
    pub fn advance_millis(&self, delta: i64) {
        self.millis.fetch_add(delta, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(self.millis.load(Ordering::SeqCst))
    }
}
