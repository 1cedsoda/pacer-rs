use super::{Reason, builder_methods};
use crate::types::{Millis, Rule, RuleResult};

type IsOpenFn<C> = dyn Fn(&C, Millis) -> bool + Send + Sync;
type NextOpenFn<C> = dyn Fn(&C, Millis) -> Millis + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Millis, Millis) -> String + Send + Sync;

/// Allow actions only inside a known window. Build with [`fixed_window`].
pub struct FixedWindow<C: ?Sized> {
    name: String,
    is_open: Box<IsOpenFn<C>>,
    next_open: Box<NextOpenFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Ready while `is_open(ctx, now)`; otherwise blocked until `next_open(ctx, now)`.
///
/// Panics if the window is closed and `next_open` is not strictly after `now`.
/// Default name `"fixed-window"`, default reason `"outside fixed window"`.
/// Reason closure args: `(ctx, now, retry_at)`.
///
/// For local-time windows see `zoned_window` (cargo feature `tz`).
pub fn fixed_window<C, O, N>(is_open: O, next_open: N) -> FixedWindow<C>
where
    C: ?Sized,
    O: Fn(&C, Millis) -> bool + Send + Sync + 'static,
    N: Fn(&C, Millis) -> Millis + Send + Sync + 'static,
{
    FixedWindow {
        name: "fixed-window".to_owned(),
        is_open: Box::new(is_open),
        next_open: Box::new(next_open),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> FixedWindow<C> {
    builder_methods!(&C, Millis, Millis);
}

impl<C: ?Sized> Rule<C> for FixedWindow<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        if (self.is_open)(ctx, now) {
            return RuleResult::Ready;
        }
        let retry_at = (self.next_open)(ctx, now);
        assert!(
            retry_at > now,
            "next_open must return a future time when the window is closed"
        );
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at),
                || "outside fixed window".to_owned(),
            ),
        )
    }
}
