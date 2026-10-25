use super::{Reason, ScopeFn, assert_non_negative, builder_methods};
use crate::types::{Millis, Rule, RuleResult};

type CountFn<C> = dyn Fn(&C, &str) -> u64 + Send + Sync;
type RetryFn<C> = dyn Fn(&C, Millis, &str, u64) -> Millis + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Millis, Millis, &str, u64) -> String + Send + Sync;

/// Limit inflight actions per scope. Build with [`concurrency_limit`].
pub struct ConcurrencyLimit<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    max: u64,
    get_inflight_count: Box<CountFn<C>>,
    retry_after: Box<RetryFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Ready while `get_inflight_count(ctx, scope) < max`; otherwise blocked until
/// `now + retry_after` (default 5 000 ms, see [`ConcurrencyLimit::retry_after_ms`]
/// and [`ConcurrencyLimit::retry_after_fn`]).
///
/// Panics if `max == 0` or the retry delay is negative. Default name
/// `"concurrency-limit"`, default reason `"<active>/<max> inflight"`. Reason
/// closure args: `(ctx, now, retry_at, scope, active)`.
pub fn concurrency_limit<C, S, G>(scope: S, max: u64, get_inflight_count: G) -> ConcurrencyLimit<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: Fn(&C, &str) -> u64 + Send + Sync + 'static,
{
    assert!(max > 0, "max must be positive");
    ConcurrencyLimit {
        name: "concurrency-limit".to_owned(),
        scope: Box::new(scope),
        max,
        get_inflight_count: Box::new(get_inflight_count),
        retry_after: Box::new(|_, _, _, _| 5_000),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> ConcurrencyLimit<C> {
    builder_methods!(&C, Millis, Millis, &str, u64);

    /// Retry after a constant delay (panics if negative).
    pub fn retry_after_ms(mut self, ms: Millis) -> Self {
        assert_non_negative(ms, "retry_after_ms");
        self.retry_after = Box::new(move |_, _, _, _| ms);
        self
    }

    /// Compute the retry delay from `(ctx, now, scope, active)`.
    pub fn retry_after_fn<F>(mut self, f: F) -> Self
    where
        F: Fn(&C, Millis, &str, u64) -> Millis + Send + Sync + 'static,
    {
        self.retry_after = Box::new(f);
        self
    }
}

impl<C: ?Sized> Rule<C> for ConcurrencyLimit<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        let scope = (self.scope)(ctx);
        let active = (self.get_inflight_count)(ctx, &scope);
        if active < self.max {
            return RuleResult::Ready;
        }
        let retry_after =
            assert_non_negative((self.retry_after)(ctx, now, &scope, active), "retry_after");
        let retry_at = now.saturating_add(retry_after);
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope, active),
                || format!("{active}/{} inflight", self.max),
            ),
        )
    }
}
