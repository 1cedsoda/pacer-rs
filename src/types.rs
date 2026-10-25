//! Core types.

use std::time::Duration;

use crate::time::Time;

/// When a blocked rule may become ready again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wait {
    /// Retry at this time.
    At(Time),
    /// Permanently blocked, never retry (TypeScript: `retryAt: null`).
    Never,
}

impl Wait {
    /// The concrete retry time, or `None` for [`Wait::Never`].
    pub fn at(self) -> Option<Time> {
        match self {
            Wait::At(t) => Some(t),
            Wait::Never => None,
        }
    }

    /// `true` for [`Wait::Never`].
    pub fn is_never(self) -> bool {
        matches!(self, Wait::Never)
    }

    /// Combine waits so that every one of them is satisfied: `Never` if any is
    /// `Never`, otherwise the latest `At`. Returns `None` for an empty iterator.
    pub fn latest<I: IntoIterator<Item = Wait>>(waits: I) -> Option<Wait> {
        waits.into_iter().fold(None, |acc, wait| {
            Some(match (acc, wait) {
                (None, w) => w,
                (Some(Wait::Never), _) | (Some(_), Wait::Never) => Wait::Never,
                (Some(Wait::At(a)), Wait::At(b)) => Wait::At(a.max(b)),
            })
        })
    }

    /// Combine waits so that any one of them is satisfied: the earliest `At`,
    /// `Never` only if every wait is `Never`. Returns `None` for an empty iterator.
    pub fn earliest<I: IntoIterator<Item = Wait>>(waits: I) -> Option<Wait> {
        waits.into_iter().fold(None, |acc, wait| {
            Some(match (acc, wait) {
                (None, w) => w,
                (Some(Wait::Never), w) | (Some(w), Wait::Never) => w,
                (Some(Wait::At(a)), Wait::At(b)) => Wait::At(a.min(b)),
            })
        })
    }
}

/// The answer of a single rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RuleResult {
    /// The rule does not block.
    Ready,
    /// The rule blocks.
    Blocked {
        /// When to retry.
        wait: Wait,
        /// Human-readable reason.
        reason: String,
    },
}

impl RuleResult {
    /// Blocked until `at`.
    pub fn blocked_until(at: Time, reason: impl Into<String>) -> Self {
        RuleResult::Blocked {
            wait: Wait::At(at),
            reason: reason.into(),
        }
    }

    /// Permanently blocked.
    pub fn blocked_forever(reason: impl Into<String>) -> Self {
        RuleResult::Blocked {
            wait: Wait::Never,
            reason: reason.into(),
        }
    }

    /// `true` for [`RuleResult::Ready`].
    pub fn is_ready(&self) -> bool {
        matches!(self, RuleResult::Ready)
    }
}

/// One blocking rule inside a [`Verdict`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Block {
    /// Name of the rule that blocked.
    pub rule: String,
    /// When that rule may become ready.
    pub wait: Wait,
    /// Human-readable reason.
    pub reason: String,
}

/// The combined answer of a [`Pacer`](crate::Pacer).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// No rule blocks.
    Ready,
    /// At least one rule blocks.
    Blocked {
        /// Latest `At` across all blocks; `Never` if any block is `Never`.
        wait: Wait,
        /// Every blocking rule, in rule order.
        blocks: Vec<Block>,
    },
}

impl Verdict {
    /// `true` for [`Verdict::Ready`].
    pub fn is_ready(&self) -> bool {
        matches!(self, Verdict::Ready)
    }

    /// The combined wait, `None` when ready.
    pub fn wait(&self) -> Option<Wait> {
        match self {
            Verdict::Ready => None,
            Verdict::Blocked { wait, .. } => Some(*wait),
        }
    }

    /// The blocks, empty when ready.
    pub fn blocks(&self) -> &[Block] {
        match self {
            Verdict::Ready => &[],
            Verdict::Blocked { blocks, .. } => blocks,
        }
    }

    /// When to act next: `Some(now)` when ready, `Some(t)` for `At(t)`, `None`
    /// for `Never`. Feed this straight into a durable timer.
    pub fn next_at(&self, now: Time) -> Option<Time> {
        match self {
            Verdict::Ready => Some(now),
            Verdict::Blocked { wait, .. } => wait.at(),
        }
    }
}

/// One pacing rule for one context type.
///
/// Rules are synchronous and should be pure over `ctx` and `now`: load all data
/// a rule needs into the context before calling [`Pacer::check`](crate::Pacer::check).
/// A rule that panics makes the pacer panic; there is no error swallowing.
pub trait Rule<C: ?Sized>: Send + Sync {
    /// Rule name, reported in [`Block::rule`].
    fn name(&self) -> &str;
    /// Decide whether the action may happen at `now`.
    fn check(&self, ctx: &C, now: Time) -> RuleResult;
}

impl<C: ?Sized, R: Rule<C> + ?Sized> Rule<C> for Box<R> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        (**self).check(ctx, now)
    }
}

impl<C: ?Sized, R: Rule<C> + ?Sized> Rule<C> for std::sync::Arc<R> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        (**self).check(ctx, now)
    }
}

/// A rule backed by a closure. Build with [`rule_fn`].
pub struct FnRule<F> {
    name: String,
    check: F,
}

/// Create a custom rule from a name and a closure.
pub fn rule_fn<C, F>(name: impl Into<String>, check: F) -> FnRule<F>
where
    C: ?Sized,
    F: Fn(&C, Time) -> RuleResult + Send + Sync,
{
    FnRule {
        name: name.into(),
        check,
    }
}

impl<C, F> Rule<C> for FnRule<F>
where
    C: ?Sized,
    F: Fn(&C, Time) -> RuleResult + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }
    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        (self.check)(ctx, now)
    }
}

/// A half-open time window `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeWindow {
    /// Inclusive start.
    pub start: Time,
    /// Exclusive end.
    pub end: Time,
}

impl TimeWindow {
    /// Create a window.
    pub fn new(start: Time, end: Time) -> Self {
        TimeWindow { start, end }
    }

    /// The window of length `span` that contains `t`, aligned to multiples
    /// of `span` on the UTC timeline (the UTC hour, the UTC day, ...). For
    /// local calendar days use `pacer::tz::day` (feature `tz`).
    ///
    /// ```
    /// use std::time::Duration;
    /// use pacer::{Time, TimeWindow};
    ///
    /// let t: Time = "2026-10-25T13:45:00Z".parse().unwrap();
    /// let hour = TimeWindow::utc_aligned(t, Duration::from_secs(3600));
    /// assert_eq!(hour.start.to_string(), "2026-10-25T13:00:00Z");
    /// assert_eq!(hour.end.to_string(), "2026-10-25T14:00:00Z");
    /// ```
    pub fn utc_aligned(t: Time, span: Duration) -> Self {
        let start = t.truncate(span);
        TimeWindow::new(start, start + span)
    }

    /// The window's length.
    pub fn len(&self) -> Duration {
        self.end.since(self.start)
    }

    /// `true` if the window holds no instant.
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// Whether `t` lies in `[start, end)`.
    pub fn contains(&self, t: Time) -> bool {
        self.start <= t && t < self.end
    }
}

/// A span that is either a constant or derived from the context.
///
/// Implemented for [`Duration`] and for any `Fn(&C) -> Duration + Send + Sync`.
pub trait SpanFn<C: ?Sized>: Send + Sync {
    /// Resolve the span for `ctx`.
    fn span(&self, ctx: &C) -> Duration;
}

impl<C: ?Sized> SpanFn<C> for Duration {
    fn span(&self, _ctx: &C) -> Duration {
        *self
    }
}

impl<C: ?Sized, F: Fn(&C) -> Duration + Send + Sync> SpanFn<C> for F {
    fn span(&self, ctx: &C) -> Duration {
        self(ctx)
    }
}
