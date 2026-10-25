//! Run a pacer through time without waiting: [`simulate`] jumps from one
//! answer to the next (ready → act → check again; blocked → jump to the
//! retry time) and records what happened. The result prints as a timeline,
//! so a test can compare a whole schedule (a cadence over a DST switch, a
//! backoff, a quota) in one readable assertion.
//!
//! ```
//! use std::time::Duration;
//! use pacer::{Pacer, Time, min_gap, sim::simulate};
//!
//! struct Sent(Vec<Time>);
//! let pacer = Pacer::new(vec![Box::new(min_gap(
//!     |_: &Sent| "s".into(),
//!     Duration::from_secs(3600),
//!     |s: &Sent, _: &str| s.0.last().copied(),
//! ))]);
//! let from: Time = "2026-01-01T09:00:00Z".parse().unwrap();
//! let run = simulate(&pacer, &mut Sent(vec![]), from, from + Duration::from_secs(3 * 3600), |s, t| s.0.push(t));
//! assert_eq!(run.to_string(), "\
//! 2026-01-01T09:00:00Z acted
//! 2026-01-01T10:00:00Z acted
//! 2026-01-01T11:00:00Z acted
//! 2026-01-01T12:00:00Z horizon
//! ");
//! ```

use std::fmt;

use crate::pacer::Pacer;
use crate::time::Time;
use crate::types::{Verdict, Wait};

/// How often a simulation may act at one instant before it assumes the
/// action does not change the context (an endless loop) and panics.
pub const MAX_ACTS_AT_ONCE: usize = 1_000;

/// One event of a [`Simulation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The pacer was ready and the action ran at this time.
    Acted(Time),
    /// The pacer blocked for good (`Wait::Never`): the run ends here.
    Stopped {
        /// When it stopped.
        at: Time,
        /// The final verdict (its blocks name the rules).
        verdict: Verdict,
    },
    /// The next retry lies at or after the horizon: the run ends.
    Horizon(Time),
}

/// What [`simulate`] recorded, in time order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Simulation {
    /// Every event.
    pub steps: Vec<Step>,
}

impl Simulation {
    /// The times the action ran.
    pub fn acted(&self) -> Vec<Time> {
        self.steps
            .iter()
            .filter_map(|s| match s {
                Step::Acted(t) => Some(*t),
                _ => None,
            })
            .collect()
    }

    /// The timeline with local times in `tz` (cargo feature `tz`).
    #[cfg(feature = "tz")]
    pub fn in_zone(&self, tz: &crate::tz::TimeZone) -> String {
        let fmt = |t: Time| crate::tz::local(tz, t).to_string();
        self.lines(&fmt)
    }

    fn lines(&self, fmt: &dyn Fn(Time) -> String) -> String {
        let mut out = String::new();
        for step in &self.steps {
            let line = match step {
                Step::Acted(t) => format!("{} acted", fmt(*t)),
                Step::Horizon(t) => format!("{} horizon", fmt(*t)),
                Step::Stopped { at, verdict } => {
                    let rules: Vec<&str> =
                        verdict.blocks().iter().map(|b| b.rule.as_str()).collect();
                    format!("{} stopped by {}", fmt(*at), rules.join(", "))
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

/// One line per step: `<time> acted`, `<time> stopped by <rules>`,
/// `<time> horizon`.
impl fmt::Display for Simulation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.lines(&|t| t.to_string()))
    }
}

/// Runs `pacer` over `[from, until)`: whenever it is ready, `act` runs at
/// that time and may change the context (record the attempt, count the
/// send); when it blocks, the clock jumps to the retry time. Ends at
/// `until` or when a rule blocks for good. Panics if the pacer stays ready
/// after [`MAX_ACTS_AT_ONCE`] actions at one instant.
pub fn simulate<C>(
    pacer: &Pacer<C>,
    ctx: &mut C,
    from: Time,
    until: Time,
    mut act: impl FnMut(&mut C, Time),
) -> Simulation {
    let mut steps = Vec::new();
    let mut now = from;
    let mut at_once = 0;
    while now < until {
        match pacer.check(ctx, now) {
            Verdict::Ready => {
                at_once += 1;
                assert!(
                    at_once <= MAX_ACTS_AT_ONCE,
                    "still ready after {MAX_ACTS_AT_ONCE} actions at {now}: does the action change the context?"
                );
                act(ctx, now);
                steps.push(Step::Acted(now));
            }
            verdict @ Verdict::Blocked {
                wait: Wait::Never, ..
            } => {
                steps.push(Step::Stopped { at: now, verdict });
                return Simulation { steps };
            }
            Verdict::Blocked {
                wait: Wait::At(at), ..
            } => {
                // A rule that answers "now" while blocking must not stall.
                let next = if at > now {
                    at
                } else {
                    Time::from_micros(now.as_micros() + 1)
                };
                if next != now {
                    at_once = 0;
                }
                now = next;
            }
        }
    }
    steps.push(Step::Horizon(until));
    Simulation { steps }
}
