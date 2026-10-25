use super::{Reason, ScopeFn, builder_methods};
use crate::time::Time;
use crate::types::{Rule, RuleResult, SpanFn};

type LastFn<C> = dyn Fn(&C, &str) -> Option<Time> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Time, Time, &str, Time) -> String + Send + Sync;

/// Pause a scope after a failure. Build with [`cooldown`].
pub struct Cooldown<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    cooldown: Box<dyn SpanFn<C>>,
    get_last_failure_time: Box<LastFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Block until `last_failure_time + cooldown`. No failure → ready.
///
/// `cooldown` is a constant [`Duration`](std::time::Duration) or `Fn(&C) -> Duration` (e.g. exponential
/// back-off on an attempt counter).
/// Default name `"cooldown"`, default reason `"cooldown is still active"`.
/// Reason closure args: `(ctx, now, retry_at, scope, last_failure_time)`.
pub fn cooldown<C, S, G, L>(scope: S, cooldown: G, get_last_failure_time: L) -> Cooldown<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: SpanFn<C> + 'static,
    L: Fn(&C, &str) -> Option<Time> + Send + Sync + 'static,
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
    builder_methods!(&C, Time, Time, &str, Time);
}

impl<C: ?Sized> Rule<C> for Cooldown<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        let cooldown = self.cooldown.span(ctx);
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
