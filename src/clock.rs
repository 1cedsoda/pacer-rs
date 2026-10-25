//! Where "now" comes from, at the edge of a program. Rules never read a
//! clock: they take `now` as a parameter, so they are pure and testable
//! without fakes. A [`Clock`] is for the code that calls them.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use crate::time::{Time, span_micros};

/// A source of the current [`Time`].
///
/// Object safe: pass `&dyn Clock` or `Arc<dyn Clock>` through your program
/// and give tests a [`ManualClock`].
pub trait Clock: Send + Sync {
    /// The current time.
    fn now(&self) -> Time;

    /// The current time in epoch milliseconds, for code that still counts
    /// milliseconds as `i64` (a bridge while migrating to [`Time`]).
    fn now_ms(&self) -> i64 {
        self.now().as_millis()
    }
}

impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> Time {
        (**self).now()
    }
}
impl<C: Clock + ?Sized> Clock for Box<C> {
    fn now(&self) -> Time {
        (**self).now()
    }
}
impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> Time {
        (**self).now()
    }
}

/// The operating system's wall clock. It can jump (NTP, manual changes):
/// right for deadlines, calendar days and anything stored.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Time {
        Time::now()
    }
}

/// Wall time anchored once, then advanced by the monotonic
/// [`Instant`]: never goes backwards and ignores jumps of the system clock.
/// Right for in-memory rate limiters and timeouts; not for calendar days
/// over long uptimes (it drifts from the wall clock as NTP corrects it).
#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    anchor: Time,
    started: Instant,
}

impl MonotonicClock {
    /// Anchored at the system time now.
    pub fn new() -> Self {
        MonotonicClock {
            anchor: Time::now(),
            started: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Time {
        self.anchor + self.started.elapsed()
    }
}

/// A clock that moves only when told to: for tests and simulations.
/// Shared by reference or `Arc`; setting it is visible to every holder.
///
/// ```
/// use std::time::Duration;
/// use pacer::{Clock, ManualClock, Time};
///
/// let clock = ManualClock::at("2026-10-25T00:00:00Z".parse().unwrap());
/// clock.advance(Duration::from_secs(3600));
/// assert_eq!(clock.now().to_string(), "2026-10-25T01:00:00Z");
/// ```
#[derive(Debug, Default)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    /// A clock standing at `t`.
    pub fn at(t: Time) -> Self {
        ManualClock(AtomicI64::new(t.as_micros()))
    }
    /// Moves the clock to `t` (also backwards).
    pub fn set(&self, t: Time) {
        self.0.store(t.as_micros(), Ordering::SeqCst);
    }
    /// Moves the clock forward by `span` (saturating).
    pub fn advance(&self, span: Duration) {
        let by = span_micros(span).unwrap_or(i64::MAX);
        let mut current = self.0.load(Ordering::SeqCst);
        while let Err(seen) = self.0.compare_exchange(
            current,
            current.saturating_add(by),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            current = seen;
        }
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Time {
        Time::from_micros(self.0.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks() {
        let manual = ManualClock::at(Time::from_secs(5));
        manual.advance(Duration::from_millis(1_500));
        assert_eq!(manual.now(), Time::from_millis(6_500));
        assert_eq!(manual.now_ms(), 6_500);
        manual.set(Time::UNIX_EPOCH);
        let shared: Arc<dyn Clock> = Arc::new(manual);
        assert_eq!(shared.now(), Time::UNIX_EPOCH);
        let mono = MonotonicClock::new();
        let (a, b) = (mono.now(), mono.now());
        assert!(b >= a);
        assert!(SystemClock.now() > Time::from_secs(1_700_000_000));
    }
}
