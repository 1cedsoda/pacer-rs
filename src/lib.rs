//! Small, synchronous pacing engine for one class of problem:
//!
//! > Can I do this now, and if not, when should I retry?
//!
//! A [`Pacer<C>`] is a flat list of [`Rule<C>`]. Each rule is a pure, synchronous
//! function over a pre-loaded context `C` and the current [`Time`]. The pacer
//! runs every rule and combines the blocks: the latest [`Wait::At`] wins, and
//! [`Wait::Never`] wins over any `At`.
//!
//! Time has types: a [`Time`] is a point (microseconds since the Unix epoch,
//! UTC), a [`std::time::Duration`] is a span. Rules never read a clock; the
//! edge of a program reads one [`Clock`] ([`SystemClock`], [`MonotonicClock`],
//! [`ManualClock`] in tests) and passes `now` down. [`sim::simulate`] runs a
//! pacer through time without waiting, [`tz`] (feature `tz`) reads times in
//! local calendars.
//!
//! See the README for the full guide.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod clock;
mod combinators;
mod pacer;
mod rules;
pub mod sim;
mod time;
mod types;
#[cfg(feature = "tz")]
pub mod tz;

pub use clock::{Clock, ManualClock, MonotonicClock, SystemClock};
pub use combinators::{AllOf, AnyOf, Named, all_of, any_of, group};
pub use pacer::Pacer;
pub use rules::*;
pub use time::{ParseTimeError, Time};
pub use types::{Block, FnRule, Rule, RuleResult, SpanFn, TimeWindow, Verdict, Wait, rule_fn};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
