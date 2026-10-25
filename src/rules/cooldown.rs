use super::{Reason, ScopeFn, assert_non_negative, builder_methods};
use crate::types::{Millis, MillisFn, Rule, RuleResult};

type LastFn<C> = dyn Fn(&C, &str) -> Option<Millis> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Millis, Millis, &str, Millis) -> String + Send + Sync;

/// Pause a scope after a failure. Build with [`cooldown`].
pub struct Cooldown<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    cooldown: Box<dyn MillisFn<C>>,
    get_last_failure_time: Box<LastFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Block until `last_failure_time + cooldown`. No failure → ready.
///
/// `cooldown` is a constant [`Millis`] or `Fn(&C) -> Millis` (e.g. exponential
/// back-off on an attempt counter); it must be non-negative (panics otherwise).
/// Default name `"cooldown"`, default reason `"cooldown is still active"`.
/// Reason closure args: `(ctx, now, retry_at, scope, last_failure_time)`.
pub fn cooldown<C, S, G, L>(scope: S, cooldown: G, get_last_failure_time: L) -> Cooldown<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: MillisFn<C> + 'static,
    L: Fn(&C, &str) -> Option<Millis> + Send + Sync + 'static,
{
    Cooldown {
        name: "cooldown".to_owned(),
        scope: Box::new(scope),
        cooldown: Box::new(cooldown),
        get_last_failure_time: Box::new(get_last_failure_time),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> Cooldown<C> {
    builder_methods!(&C, Millis, Millis, &str, Millis);
}

impl<C: ?Sized> Rule<C> for Cooldown<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        let cooldown = assert_non_negative(self.cooldown.millis(ctx), "cooldown");
        let scope = (self.scope)(ctx);
        let Some(last) = (self.get_last_failure_time)(ctx, &scope) else {
            return RuleResult::Ready;
        };
        let retry_at = last.saturating_add(cooldown);
        if now >= retry_at {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope, last),
                || "cooldown is still active".to_owned(),
            ),
        )
    }
}
