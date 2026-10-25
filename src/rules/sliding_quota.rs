use super::{Reason, ScopeFn, builder_methods};
use crate::types::{Millis, Rule, RuleResult};

type EventsFn<C> = dyn Fn(&C, &str, Millis, Millis) -> Vec<Millis> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Millis, Millis, &str, usize, Millis) -> String + Send + Sync;

/// Quota over a rolling window. Build with [`sliding_quota`].
pub struct SlidingQuota<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    limit: usize,
    window_ms: Millis,
    get_event_times: Box<EventsFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// At most `limit` events in any rolling `window_ms`.
///
/// `get_event_times(ctx, scope, since, until)` is called with
/// `since = now - window_ms`, `until = now`; events outside `(since, now]` are
/// ignored, order does not matter. When `count >= limit`, blocks until the
/// `limit`-th newest event ages out: `events[count - limit] + window_ms`.
///
/// Panics if `limit == 0` or `window_ms <= 0`. Default name `"sliding-quota"`,
/// default reason `"sliding quota exhausted (<count>/<limit>)"`. Reason closure
/// args: `(ctx, now, retry_at, scope, count, oldest_blocking_event)`.
pub fn sliding_quota<C, S, G>(
    scope: S,
    limit: usize,
    window_ms: Millis,
    get_event_times: G,
) -> SlidingQuota<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: Fn(&C, &str, Millis, Millis) -> Vec<Millis> + Send + Sync + 'static,
{
    assert!(limit > 0, "limit must be positive");
    assert!(window_ms > 0, "window_ms must be positive");
    SlidingQuota {
        name: "sliding-quota".to_owned(),
        scope: Box::new(scope),
        limit,
        window_ms,
        get_event_times: Box::new(get_event_times),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> SlidingQuota<C> {
    builder_methods!(&C, Millis, Millis, &str, usize, Millis);
}

impl<C: ?Sized> Rule<C> for SlidingQuota<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        let scope = (self.scope)(ctx);
        let since = now - self.window_ms;
        let mut events: Vec<Millis> = (self.get_event_times)(ctx, &scope, since, now)
            .into_iter()
            .filter(|&t| t > since && t <= now)
            .collect();
        if events.len() < self.limit {
            return RuleResult::Ready;
        }
        events.sort_unstable();
        let count = events.len();
        let oldest_blocking = events[count - self.limit];
        let retry_at = oldest_blocking + self.window_ms;
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope, count, oldest_blocking),
                || format!("sliding quota exhausted ({count}/{})", self.limit),
            ),
        )
    }
}
