//! Small, synchronous pacing engine for one class of problem:
//!
//! > Can I do this now, and if not, when should I retry?
//!
//! A [`Pacer<C>`] is a flat list of [`Rule<C>`]. Each rule is a pure, synchronous
//! function over a pre-loaded context `C` and the current time (`i64` epoch
//! milliseconds). The pacer runs every rule and combines the blocks: the latest
//! [`Wait::At`] wins, and [`Wait::Never`] wins over any `At`.
//!
//! See the README for the full guide.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod combinators;
mod helpers;
mod pacer;
mod rules;
mod types;

pub use combinators::{AllOf, AnyOf, Named, all_of, any_of, group};
pub use helpers::{
    DAY_MS, HOUR_MS, MINUTE_MS, SECOND_MS, add_ms, create_window, end_of_day, end_of_hour,
    start_of_day, start_of_hour, to_iso_string,
};
pub use pacer::Pacer;
pub use rules::*;
pub use types::{
    Block, FnRule, Millis, MillisFn, Rule, RuleResult, TimeWindow, Verdict, Wait, rule_fn,
};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
