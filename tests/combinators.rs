mod common;

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{blocked_rule, never_rule, ready_rule, ts};
use pacer::{Block, Pacer, RuleResult, Verdict, Wait, all_of, any_of, group, rule_fn};

fn single_block(rule: &str, wait: Wait, reason: &str) -> Verdict {
    Verdict::Blocked {
        wait,
        blocks: vec![Block {
            rule: rule.into(),
            wait,
            reason: reason.into(),
        }],
    }
}

// ─── any_of ──────────────────────────────────────────────────────────────────

#[test]
fn any_of_returns_ready_when_one_branch_is_ready() {
    let pacer: Pacer<()> = Pacer::new(vec![Box::new(any_of(
        "branch",
        vec![
            Box::new(blocked_rule("one", "2026-01-01T10:05:00.000Z", "blocked")),
            Box::new(ready_rule("two")),
        ],
    ))]);
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        Verdict::Ready
    );
}

#[test]
fn any_of_returns_the_earliest_retry_at_when_all_branches_are_blocked() {
    let pacer: Pacer<()> = Pacer::new(vec![Box::new(any_of(
        "branch",
        vec![
            Box::new(blocked_rule(
                "one",
                "2026-01-01T10:05:00.000Z",
                "blocked one",
            )),
            Box::new(blocked_rule(
                "two",
                "2026-01-01T10:03:00.000Z",
                "blocked two",
            )),
        ],
    ))]);
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        single_block(
            "branch",
            Wait::At(ts("2026-01-01T10:03:00.000Z")),
            "all branches blocked"
        )
    );
}

/// Port of "runs all branches in parallel": Rust rules are synchronous, but
/// every branch is still evaluated (no short-circuit), like `Promise.all`.
#[test]
fn any_of_evaluates_every_branch() {
    let order = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    let tracked = |name: &'static str| {
        let order = order.clone();
        Box::new(rule_fn(name, move |_: &(), _| {
            order.lock().unwrap().push(name.to_owned());
            RuleResult::Ready
        })) as Box<dyn pacer::Rule<()>>
    };
    let pacer = Pacer::new(vec![Box::new(any_of(
        "par",
        vec![tracked("a"), tracked("b"), tracked("c")],
    )) as Box<dyn pacer::Rule<()>>]);
    pacer.check(&(), 0);

    let mut seen = order.lock().unwrap().clone();
    seen.sort();
    assert_eq!(seen, ["a", "b", "c"]);
}

#[test]
fn any_of_returns_never_when_all_branches_are_permanently_blocked() {
    let pacer: Pacer<()> = Pacer::new(vec![Box::new(any_of(
        "branch",
        vec![
            Box::new(never_rule("one", "perm-blocked")),
            Box::new(never_rule("two", "also-perm-blocked")),
        ],
    ))]);
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        single_block("branch", Wait::Never, "all branches blocked")
    );
}

#[test]
fn any_of_returns_earliest_concrete_retry_at_when_only_some_branches_are_permanently_blocked() {
    let pacer: Pacer<()> = Pacer::new(vec![Box::new(any_of(
        "branch",
        vec![
            Box::new(never_rule("one", "perm-blocked")),
            Box::new(blocked_rule("two", "2026-01-01T10:03:00.000Z", "timed")),
        ],
    ))]);
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        single_block(
            "branch",
            Wait::At(ts("2026-01-01T10:03:00.000Z")),
            "all branches blocked"
        )
    );
}

#[test]
fn any_of_with_no_branches_is_ready() {
    let rule = any_of::<()>("empty", vec![]);
    assert_eq!(pacer::Rule::check(&rule, &(), 0), RuleResult::Ready);
}

// ─── all_of ──────────────────────────────────────────────────────────────────

#[test]
fn all_of_returns_the_latest_retry_at_across_nested_rules() {
    let pacer: Pacer<()> = Pacer::new(vec![Box::new(all_of(
        "nested",
        vec![
            Box::new(blocked_rule(
                "one",
                "2026-01-01T10:02:00.000Z",
                "blocked one",
            )),
            Box::new(blocked_rule(
                "two",
                "2026-01-01T10:04:00.000Z",
                "blocked two",
            )),
        ],
    ))]);
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        single_block(
            "nested",
            Wait::At(ts("2026-01-01T10:04:00.000Z")),
            "2 rule(s) blocked"
        )
    );
}

#[test]
fn all_of_returns_never_when_any_nested_rule_is_permanently_blocked() {
    let pacer: Pacer<()> = Pacer::new(vec![Box::new(all_of(
        "nested",
        vec![
            Box::new(blocked_rule("one", "2026-01-01T10:04:00.000Z", "timed")),
            Box::new(never_rule("two", "perm-blocked")),
        ],
    ))]);
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        single_block("nested", Wait::Never, "2 rule(s) blocked")
    );
}

#[test]
fn all_of_is_ready_when_every_child_is_ready() {
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let rule = all_of::<()>(
        "nested",
        vec![
            Box::new(ready_rule("a")),
            Box::new(rule_fn("b", move |_: &(), _| {
                c.fetch_add(1, Ordering::SeqCst);
                RuleResult::Ready
            })),
        ],
    );
    assert_eq!(pacer::Rule::check(&rule, &(), 0), RuleResult::Ready);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// ─── group ───────────────────────────────────────────────────────────────────

#[test]
fn group_prefixes_rule_names_without_changing_behavior() {
    let pacer: Pacer<()> = Pacer::new(group(
        "email",
        vec![Box::new(blocked_rule(
            "min-gap",
            "2026-01-01T10:02:00.000Z",
            "blocked",
        ))],
    ));
    assert_eq!(
        pacer.check(&(), ts("2026-01-01T10:00:00.000Z")),
        single_block(
            "email/min-gap",
            Wait::At(ts("2026-01-01T10:02:00.000Z")),
            "blocked"
        )
    );
}
