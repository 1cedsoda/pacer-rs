use super::{Reason, ScopeFn, builder_methods};
use crate::helpers::to_iso_string;
use crate::types::{Millis, Rule, RuleResult};

type CompletedFn<C> = dyn Fn(&C, &str) -> Option<Millis> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, &str, Millis) -> String + Send + Sync;

/// Process an item only once, ever. Build with [`dedupe`].
pub struct Dedupe<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    get_completed_at: Box<CompletedFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Ready while `get_completed_at` returns `None`; permanently blocked
/// ([`Wait::Never`](crate::Wait::Never)) once it returns a completion time.
///
/// Default name `"dedupe"`, default reason
/// `"already processed (<ISO completion time>)"`. Reason closure args:
/// `(ctx, scope, completed_at)`.
pub fn dedupe<C, S, G>(scope: S, get_completed_at: G) -> Dedupe<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    G: Fn(&C, &str) -> Option<Millis> + Send + Sync + 'static,
{
    Dedupe {
        name: "dedupe".to_owned(),
        scope: Box::new(scope),
        get_completed_at: Box::new(get_completed_at),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> Dedupe<C> {
    builder_methods!(&C, &str, Millis);
}

impl<C: ?Sized> Rule<C> for Dedupe<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, _now: Millis) -> RuleResult {
        let scope = (self.scope)(ctx);
        let Some(completed_at) = (self.get_completed_at)(ctx, &scope) else {
            return RuleResult::Ready;
        };
        RuleResult::blocked_forever(self.reason.resolve(
            |f| f(ctx, &scope, completed_at),
            || format!("already processed ({})", to_iso_string(completed_at)),
        ))
    }
}
