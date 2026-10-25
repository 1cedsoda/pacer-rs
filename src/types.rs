//! Core types.

/// A point in time or a duration, in milliseconds. Points in time are Unix epoch
/// milliseconds (UTC).
pub type Millis = i64;

/// When a blocked rule may become ready again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wait {
    /// Retry at this epoch-millisecond timestamp.
    At(Millis),
    /// Permanently blocked, never retry (TypeScript: `retryAt: null`).
    Never,
}

impl Wait {
    /// The concrete retry time, or `None` for [`Wait::Never`].
    pub fn at(self) -> Option<Millis> {
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
    pub fn blocked_until(at: Millis, reason: impl Into<String>) -> Self {
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
    pub fn next_at(&self, now: Millis) -> Option<Millis> {
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
    /// Decide whether the action may happen at `now` (epoch ms).
    fn check(&self, ctx: &C, now: Millis) -> RuleResult;
}

impl<C: ?Sized, R: Rule<C> + ?Sized> Rule<C> for Box<R> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        (**self).check(ctx, now)
    }
}

impl<C: ?Sized, R: Rule<C> + ?Sized> Rule<C> for std::sync::Arc<R> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
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
    F: Fn(&C, Millis) -> RuleResult + Send + Sync,
{
    FnRule {
        name: name.into(),
        check,
    }
}

impl<C, F> Rule<C> for FnRule<F>
where
    C: ?Sized,
    F: Fn(&C, Millis) -> RuleResult + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }
    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        (self.check)(ctx, now)
    }
}

/// A half-open time window `[start, end)` in epoch milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeWindow {
    /// Inclusive start.
    pub start: Millis,
    /// Exclusive end.
    pub end: Millis,
}

impl TimeWindow {
    /// Create a window.
    pub fn new(start: Millis, end: Millis) -> Self {
        TimeWindow { start, end }
    }
}

/// A millisecond amount that is either a constant or derived from the context.
///
/// Implemented for [`Millis`] and for any `Fn(&C) -> Millis + Send + Sync`.
pub trait MillisFn<C: ?Sized>: Send + Sync {
    /// Resolve the amount for `ctx`.
    fn millis(&self, ctx: &C) -> Millis;
}

impl<C: ?Sized> MillisFn<C> for Millis {
    fn millis(&self, _ctx: &C) -> Millis {
        *self
    }
}

impl<C: ?Sized, F: Fn(&C) -> Millis + Send + Sync> MillisFn<C> for F {
    fn millis(&self, ctx: &C) -> Millis {
        self(ctx)
    }
}
