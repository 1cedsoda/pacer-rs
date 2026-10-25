//! Built-in rule factories.
//!
//! Every factory takes closure accessors over the (pre-loaded) context and
//! returns a concrete rule type with builder methods `named(..)`,
//! `with_reason(..)` and `with_reason_fn(..)`.

mod backoff;
mod concurrency_limit;
mod cooldown;
mod dedupe;
mod fixed_quota;
mod fixed_window;
mod gcra;
mod min_gap;
mod not_after;
mod not_before;
mod sliding_quota;
#[cfg(feature = "tz")]
mod zoned_window;

pub use backoff::{Backoff, backoff};
pub use concurrency_limit::{ConcurrencyLimit, concurrency_limit};
pub use cooldown::{Cooldown, cooldown};
pub use dedupe::{Dedupe, dedupe};
pub use fixed_quota::{FixedQuota, fixed_quota};
pub use fixed_window::{FixedWindow, fixed_window};
pub use gcra::{Gcra, gcra, gcra_spend};
pub use min_gap::{MinGap, min_gap};
pub use not_after::{NotAfter, not_after};
pub use not_before::{NotBefore, not_before};
pub use sliding_quota::{SlidingQuota, sliding_quota};
#[cfg(feature = "tz")]
pub use zoned_window::{ZonedWindow, zoned_window};

/// Scope accessor: `Fn(&C) -> String`.
pub(crate) type ScopeFn<C> = Box<dyn Fn(&C) -> String + Send + Sync>;

/// How a rule builds its block reason.
pub(crate) enum Reason<F: ?Sized> {
    Default,
    Fixed(String),
    With(Box<F>),
}

impl<F: ?Sized> Reason<F> {
    /// Resolve: fixed string, closure result via `call`, or `default()`.
    pub(crate) fn resolve(
        &self,
        call: impl FnOnce(&F) -> String,
        default: impl FnOnce() -> String,
    ) -> String {
        match self {
            Reason::Default => default(),
            Reason::Fixed(s) => s.clone(),
            Reason::With(f) => call(f),
        }
    }
}

/// Builder methods shared by every rule struct.
macro_rules! builder_methods {
    ($($arg:ty),*) => {
        /// Override the rule name.
        pub fn named(mut self, name: impl Into<String>) -> Self {
            self.name = name.into();
            self
        }

        /// Use a fixed block reason.
        pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
            self.reason = crate::rules::Reason::Fixed(reason.into());
            self
        }

        /// Compute the block reason with a closure.
        pub fn with_reason_fn<F>(mut self, reason: F) -> Self
        where
            F: Fn($($arg),*) -> String + Send + Sync + 'static,
        {
            self.reason = crate::rules::Reason::With(Box::new(reason));
            self
        }
    };
}
pub(crate) use builder_methods;

pub(crate) fn assert_non_negative(value: i64, label: &str) -> i64 {
    assert!(value >= 0, "{label} must be non-negative, got {value}");
    value
}
