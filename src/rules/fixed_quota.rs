use super::{Reason, ScopeFn, builder_methods};
use crate::time::Time;
use crate::types::{Rule, RuleResult, TimeWindow};

type WindowFn<C> = dyn Fn(&C, Time) -> TimeWindow + Send + Sync;
type CountFn<C> = dyn Fn(&C, &str, TimeWindow) -> u64 + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Time, Time, &str, u64, TimeWindow) -> String + Send + Sync;

/// Quota that resets on fixed boundaries. Build with [`fixed_quota`].
pub struct FixedQuota<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    limit: u64,
    window: Box<WindowFn<C>>,
    get_count: Box<CountFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Ready while `get_count(ctx, scope, window) < limit`; otherwise blocked until
/// `window.end`.
///
/// Panics if `limit == 0`, or at check time if `window.end <= window.start`.
/// Default name `"fixed-quota"`, default reason
/// `"fixed quota exhausted (<count>/<limit>)"`. Reason closure args:
/// `(ctx, now, retry_at, scope, count, window)`.
pub fn fixed_quota<C, S, W, G>(scope: S, limit: u64, window: W, get_count: G) -> FixedQuota<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    W: Fn(&C, Time) -> TimeWindow + Send + Sync + 'static,
    G: Fn(&C, &str, TimeWindow) -> u64 + Send + Sync + 'static,
{
    assert!(limit > 0, "limit must be positive");
    FixedQuota {
        name: "fixed-quota".to_owned(),
        scope: Box::new(scope),
        limit,
        window: Box::new(window),
        get_count: Box::new(get_count),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> FixedQuota<C> {
    builder_methods!(&C, Time, Time, &str, u64, TimeWindow);
}

impl<C: ?Sized> Rule<C> for FixedQuota<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        let scope = (self.scope)(ctx);
        let window = (self.window)(ctx, now);
        assert!(
            window.end > window.start,
            "window.end must be after window.start"
        );
        let count = (self.get_count)(ctx, &scope, window);
        if count < self.limit {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(
            window.end,
            self.reason.resolve(
                |f| f(ctx, now, window.end, &scope, count, window),
                || format!("fixed quota exhausted ({count}/{})", self.limit),
            ),
        )
    }
}
