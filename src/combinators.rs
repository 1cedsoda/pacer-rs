use crate::time::Time;
use crate::types::{Rule, RuleResult, Wait};

/// A rule with a prefixed name. Produced by [`group`].
pub struct Named<C: ?Sized> {
    name: String,
    inner: Box<dyn Rule<C>>,
}

impl<C: ?Sized> Rule<C> for Named<C> {
    fn name(&self) -> &str {
        &self.name
    }
    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        self.inner.check(ctx, now)
    }
}

/// Pure naming helper: prefixes every rule name with `"{name}/"`. Behaviour is
/// unchanged.
pub fn group<C: ?Sized + 'static>(
    name: &str,
    rules: Vec<Box<dyn Rule<C>>>,
) -> Vec<Box<dyn Rule<C>>> {
    rules
        .into_iter()
        .map(|inner| {
            Box::new(Named {
                name: format!("{name}/{}", inner.name()),
                inner,
            }) as Box<dyn Rule<C>>
        })
        .collect()
}

/// Ready if every child is ready. Build with [`all_of`].
pub struct AllOf<C: ?Sized> {
    name: String,
    rules: Vec<Box<dyn Rule<C>>>,
}

/// Blocked if any child blocks. The wait is the latest `At`, or `Never` if any
/// child is `Never`. Reason: `"{n} rule(s) blocked"`. Empty → ready.
pub fn all_of<C: ?Sized>(name: impl Into<String>, rules: Vec<Box<dyn Rule<C>>>) -> AllOf<C> {
    AllOf {
        name: name.into(),
        rules,
    }
}

impl<C: ?Sized> Rule<C> for AllOf<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        let waits: Vec<Wait> = self
            .rules
            .iter()
            .filter_map(|rule| match rule.check(ctx, now) {
                RuleResult::Ready => None,
                RuleResult::Blocked { wait, .. } => Some(wait),
            })
            .collect();
        match Wait::latest(waits.iter().copied()) {
            None => RuleResult::Ready,
            Some(wait) => RuleResult::Blocked {
                wait,
                reason: format!("{} rule(s) blocked", waits.len()),
            },
        }
    }
}

/// Ready if any child is ready. Build with [`any_of`].
pub struct AnyOf<C: ?Sized> {
    name: String,
    rules: Vec<Box<dyn Rule<C>>>,
}

/// Ready if any child is ready (every child is evaluated). When all block, the
/// wait is the earliest `At`; `Never` only if every child is `Never`. Reason:
/// `"all branches blocked"`. Empty → ready.
pub fn any_of<C: ?Sized>(name: impl Into<String>, rules: Vec<Box<dyn Rule<C>>>) -> AnyOf<C> {
    AnyOf {
        name: name.into(),
        rules,
    }
}

impl<C: ?Sized> Rule<C> for AnyOf<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        if self.rules.is_empty() {
            return RuleResult::Ready;
        }
        // Evaluate every branch (like the TypeScript Promise.all), then decide.
        let results: Vec<RuleResult> = self.rules.iter().map(|r| r.check(ctx, now)).collect();
        if results.iter().any(RuleResult::is_ready) {
            return RuleResult::Ready;
        }
        let wait = Wait::earliest(results.iter().filter_map(|r| match r {
            RuleResult::Ready => None,
            RuleResult::Blocked { wait, .. } => Some(*wait),
        }))
        .unwrap_or(Wait::Never);
        RuleResult::Blocked {
            wait,
            reason: "all branches blocked".to_owned(),
        }
    }
}
