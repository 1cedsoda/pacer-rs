mod common;

use common::{blocked_rule, ready_rule, ts};
use pacer::{Block, Millis, Pacer, Rule, RuleResult, Verdict, Wait, rule_fn};

#[test]
fn returns_ready_when_every_rule_is_ready() {
    let pacer = Pacer::new(vec![
        Box::new(ready_rule("one")),
        Box::new(ready_rule("two")),
    ]);
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        Verdict::Ready
    );
}

#[test]
fn returns_the_latest_retry_at_across_blocking_rules() {
    let pacer: Pacer<()> = Pacer::new(vec![
        Box::new(blocked_rule(
            "one",
            "2026-01-01T10:05:00.000Z",
            "blocked one",
        )),
        Box::new(blocked_rule(
            "two",
            "2026-01-01T10:10:00.000Z",
            "blocked two",
        )),
        Box::new(ready_rule("three")),
    ]);

    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        Verdict::Blocked {
            wait: Wait::At(ts("2026-01-01T10:10:00.000Z")),
            blocks: vec![
                Block {
                    rule: "one".into(),
                    wait: Wait::At(ts("2026-01-01T10:05:00.000Z")),
                    reason: "blocked one".into(),
                },
                Block {
                    rule: "two".into(),
                    wait: Wait::At(ts("2026-01-01T10:10:00.000Z")),
                    reason: "blocked two".into(),
                },
            ],
        }
    );
}

/// Port of "supports async and sync rules together": in Rust every rule is
/// synchronous, so this mixes a closure rule with a hand-written struct rule.
#[test]
fn supports_closure_and_struct_rules_together() {
    struct StructRule;
    impl Rule<()> for StructRule {
        fn name(&self) -> &str {
            "struct"
        }
        fn check(&self, _: &(), _: Millis) -> RuleResult {
            RuleResult::blocked_until(ts("2026-01-01T10:03:00.000Z"), "struct block")
        }
    }

    let pacer: Pacer<()> = Pacer::new(vec![Box::new(ready_rule("closure")), Box::new(StructRule)]);

    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        Verdict::Blocked {
            wait: Wait::At(ts("2026-01-01T10:03:00.000Z")),
            blocks: vec![Block {
                rule: "struct".into(),
                wait: Wait::At(ts("2026-01-01T10:03:00.000Z")),
                reason: "struct block".into(),
            }],
        }
    );
}

#[test]
#[should_panic(expected = "boom")]
fn propagates_rule_panics() {
    let pacer: Pacer<()> = Pacer::new(vec![
        Box::new(ready_rule("safe")),
        Box::new(rule_fn("throws", |_: &(), _| -> RuleResult {
            panic!("boom")
        })),
    ]);
    pacer.check(&(), ts("2026-01-01T10:00:00.000Z"));
}

#[test]
fn returns_never_when_any_rule_is_permanently_blocked() {
    let pacer: Pacer<()> = Pacer::new(vec![
        Box::new(blocked_rule("one", "2026-01-01T10:05:00.000Z", "timed")),
        Box::new(rule_fn("never", |_: &(), _| {
            RuleResult::blocked_forever("permanent")
        })),
    ]);

    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        Verdict::Blocked {
            wait: Wait::Never,
            blocks: vec![
                Block {
                    rule: "one".into(),
                    wait: Wait::At(ts("2026-01-01T10:05:00.000Z")),
                    reason: "timed".into(),
                },
                Block {
                    rule: "never".into(),
                    wait: Wait::Never,
                    reason: "permanent".into(),
                },
            ],
        }
    );
}

#[test]
fn empty_pacer_is_ready() {
    let pacer: Pacer<()> = Pacer::new(vec![]);
    assert_eq!(pacer.check(&(), 0), Verdict::Ready);
}

#[test]
fn next_at_maps_verdicts_to_timer_targets() {
    let now = ts("2026-01-01T10:00:00.000Z");
    assert_eq!(Verdict::Ready.next_at(now), Some(now));

    let at = Verdict::Blocked {
        wait: Wait::At(now + 5_000),
        blocks: vec![],
    };
    assert_eq!(at.next_at(now), Some(now + 5_000));

    let never = Verdict::Blocked {
        wait: Wait::Never,
        blocks: vec![],
    };
    assert_eq!(never.next_at(now), None);
}

#[test]
fn works_with_unsized_contexts() {
    let pacer: Pacer<str> = Pacer::new(vec![Box::new(rule_fn("len", |ctx: &str, now| {
        if ctx.len() < 3 {
            RuleResult::Ready
        } else {
            RuleResult::blocked_until(now + 1, "too long")
        }
    }))]);
    assert!(pacer.check("ab", 0).is_ready());
    assert_eq!(pacer.check("abc", 0).next_at(0), Some(1));
}
