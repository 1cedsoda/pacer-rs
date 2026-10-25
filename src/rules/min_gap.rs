use super::{Reason, ScopeFn, assert_non_negative, builder_methods};
use crate::types::{Millis, MillisFn, Rule, RuleResult};

type LastFn<C> = dyn Fn(&C, &str) -> Option<Millis> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Millis, Millis, &str, Millis) -> String + Send + Sync;

/// Minimum delay between actions in one scope. Build with [`min_gap`].
pub struct MinGap<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    gap: Box<dyn MillisFn<C>>,
    get_last_event_time: Box<LastFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Block until `last_event_time + gap`. No last event → ready.
///
/// `gap` is a constant [`Millis`] or `Fn(&C) -> Millis`; it must be
/// non-negative (panics otherwise). Default name `"min-gap"`, default reason
/// `"minimum gap has not elapsed"`. Reason closure args:
/// `(ctx, now, retry_at, scope, last_event_time)`.
pub fn min_gap<C, S, G, L>(scope: S, gap: G, get_last_event_time: L) -> MinGap<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: MillisFn<C> + 'static,
    L: Fn(&C, &str) -> Option<Millis> + Send + Sync + 'static,
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
    builder_methods!(&C, Millis, Millis, &str, Millis);
}

impl<C: ?Sized> Rule<C> for MinGap<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        let gap = assert_non_negative(self.gap.millis(ctx), "gap");
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
