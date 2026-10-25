use super::{Reason, ScopeFn, builder_methods};
use std::time::Duration;

use crate::time::Time;
use crate::types::{Rule, RuleResult};

type EventsFn<C> = dyn Fn(&C, &str, Time, Time) -> Vec<Time> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Time, Time, &str, usize, Time) -> String + Send + Sync;

/// Quota over a rolling window. Build with [`sliding_quota`].
pub struct SlidingQuota<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    limit: usize,
    window: Duration,
    get_event_times: Box<EventsFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// At most `limit` events in any rolling `window`.
///
/// `get_event_times(ctx, scope, since, until)` is called with
/// `since = now - window`, `until = now`; events outside `(since, now]` are
/// ignored, order does not matter. When `count >= limit`, blocks until the
/// `limit`-th newest event ages out: `events[count - limit] + window`.
///
/// Panics if `limit == 0` or `window` is zero. Default name `"sliding-quota"`,
/// default reason `"sliding quota exhausted (<count>/<limit>)"`. Reason closure
/// args: `(ctx, now, retry_at, scope, count, oldest_blocking_event)`.
pub fn sliding_quota<C, S, G>(
    scope: S,
    limit: usize,
    window: Duration,
    get_event_times: G,
) -> SlidingQuota<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: Fn(&C, &str, Time, Time) -> Vec<Time> + Send + Sync + 'static,
{
    assert!(limit > 0, "limit must be positive");
    assert!(!window.is_zero(), "window must be positive");
    SlidingQuota {
        name: "sliding-quota".to_owned(),
        scope: Box::new(scope),
        limit,
        window,
        get_event_times: Box::new(get_event_times),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> SlidingQuota<C> {
    builder_methods!(&C, Time, Time, &str, usize, Time);
}

impl<C: ?Sized> Rule<C> for SlidingQuota<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        let scope = (self.scope)(ctx);
        let since = now - self.window;
        let mut events: Vec<Time> = (self.get_event_times)(ctx, &scope, since, now)
            .into_iter()
            .filter(|&t| t > since && t <= now)
            .collect();
        if events.len() < self.limit {
            return RuleResult::Ready;
        }
        events.sort_unstable();
        let count = events.len();
        let oldest_blocking = events[count - self.limit];
        let retry_at = oldest_blocking + self.window;
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope, count, oldest_blocking),
                || format!("sliding quota exhausted ({count}/{})", self.limit),
            ),
        )
    }
}
