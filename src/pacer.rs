use crate::types::{Block, Millis, Rule, RuleResult, Verdict, Wait};

/// The engine that aggregates rules. Stateless: the same rules, context and
/// `now` always produce the same verdict.
pub struct Pacer<C: ?Sized> {
    rules: Vec<Box<dyn Rule<C>>>,
}

impl<C: ?Sized> Pacer<C> {
    /// Build a pacer from a flat list of rules.
    pub fn new(rules: Vec<Box<dyn Rule<C>>>) -> Self {
        Pacer { rules }
    }

    /// The rules, in evaluation order.
    pub fn rules(&self) -> &[Box<dyn Rule<C>>] {
        &self.rules
    }

    /// Run every rule and combine the blocks: the latest `At` wins, `Never`
    /// wins over any `At`.
    pub fn check(&self, ctx: &C, now: Millis) -> Verdict {
        let blocks: Vec<Block> = self
            .rules
            .iter()
            .filter_map(|rule| match rule.check(ctx, now) {
                RuleResult::Ready => None,
                RuleResult::Blocked { wait, reason } => Some(Block {
                    rule: rule.name().to_owned(),
                    wait,
                    reason,
                }),
            })
            .collect();

        match Wait::latest(blocks.iter().map(|b| b.wait)) {
            None => Verdict::Ready,
            Some(wait) => Verdict::Blocked { wait, blocks },
        }
    }
}

impl<C: ?Sized> FromIterator<Box<dyn Rule<C>>> for Pacer<C> {
    fn from_iter<I: IntoIterator<Item = Box<dyn Rule<C>>>>(iter: I) -> Self {
        Pacer::new(iter.into_iter().collect())
    }
}
