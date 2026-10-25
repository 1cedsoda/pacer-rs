use super::{Reason, builder_methods};
use crate::types::{Millis, Rule, RuleResult};

type TimeFn<C> = dyn Fn(&C) -> Millis + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Millis, Millis) -> String + Send + Sync;

/// Do not start before a scheduled time. Build with [`not_before`].
pub struct NotBefore<C: ?Sized> {
    name: String,
    get_time: Box<TimeFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Blocked until `get_time(ctx)`, ready from then on (inclusive).
///
/// Default name `"not-before"`, default reason
/// `"not ready before the scheduled time"`. Reason closure args:
/// `(ctx, now, retry_at)`.
pub fn not_before<C, T>(get_time: T) -> NotBefore<C>
where
    C: ?Sized,
    T: Fn(&C) -> Millis + Send + Sync + 'static,
{
    NotBefore {
        name: "not-before".to_owned(),
        get_time: Box::new(get_time),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> NotBefore<C> {
    builder_methods!(&C, Millis, Millis);
}

impl<C: ?Sized> Rule<C> for NotBefore<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        let retry_at = (self.get_time)(ctx);
        if now >= retry_at {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at),
                || "not ready before the scheduled time".to_owned(),
            ),
        )
    }
}
