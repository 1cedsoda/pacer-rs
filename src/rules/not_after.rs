use super::{Reason, builder_methods};
use crate::time::Time;
use crate::types::{Rule, RuleResult};

type DeadlineFn<C> = dyn Fn(&C) -> Option<Time> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Time, Time) -> String + Send + Sync;

/// Never act after a deadline. Build with [`not_after`].
pub struct NotAfter<C: ?Sized> {
    name: String,
    get_deadline: Box<DeadlineFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Ready up to and including the deadline; permanently blocked
/// ([`Wait::Never`](crate::Wait::Never)) once `now > deadline`. No deadline
/// (`None`) → ready.
///
/// Default name `"not-after"`, default reason `"too late"`. Reason closure
/// args: `(ctx, now, deadline)`.
pub fn not_after<C, D>(get_deadline: D) -> NotAfter<C>
where
    C: ?Sized,
    D: Fn(&C) -> Option<Time> + Send + Sync + 'static,
{
    NotAfter {
        name: "not-after".to_owned(),
        get_deadline: Box::new(get_deadline),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> NotAfter<C> {
    builder_methods!(&C, Time, Time);
}

impl<C: ?Sized> Rule<C> for NotAfter<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        match (self.get_deadline)(ctx) {
            Some(deadline) if now > deadline => RuleResult::blocked_forever(
                self.reason
                    .resolve(|f| f(ctx, now, deadline), || "too late".to_owned()),
            ),
            _ => RuleResult::Ready,
        }
    }
}
