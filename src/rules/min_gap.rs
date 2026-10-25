use super::{Reason, ScopeFn, builder_methods};
use crate::time::Time;
use crate::types::{Rule, RuleResult, SpanFn};

type LastFn<C> = dyn Fn(&C, &str) -> Option<Time> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Time, Time, &str, Time) -> String + Send + Sync;

/// Minimum delay between actions in one scope. Build with [`min_gap`].
pub struct MinGap<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    gap: Box<dyn SpanFn<C>>,
    get_last_event_time: Box<LastFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Block until `last_event_time + gap`. No last event → ready.
///
/// `gap` is a constant [`Duration`](std::time::Duration) or `Fn(&C) -> Duration`. Default name `"min-gap"`, default reason
/// `"minimum gap has not elapsed"`. Reason closure args:
/// `(ctx, now, retry_at, scope, last_event_time)`.
pub fn min_gap<C, S, G, L>(scope: S, gap: G, get_last_event_time: L) -> MinGap<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: SpanFn<C> + 'static,
    L: Fn(&C, &str) -> Option<Time> + Send + Sync + 'static,
{
    MinGap {
        name: "min-gap".to_owned(),
        scope: Box::new(scope),
        gap: Box::new(gap),
        get_last_event_time: Box::new(get_last_event_time),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> MinGap<C> {
    builder_methods!(&C, Time, Time, &str, Time);
}

impl<C: ?Sized> Rule<C> for MinGap<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        let gap = self.gap.span(ctx);
        let scope = (self.scope)(ctx);
        let Some(last) = (self.get_last_event_time)(ctx, &scope) else {
            return RuleResult::Ready;
        };
        let retry_at = last.saturating_add(gap);
        if now >= retry_at {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope, last),
                || "minimum gap has not elapsed".to_owned(),
            ),
        )
    }
}
