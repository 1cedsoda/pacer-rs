mod common;

use common::ts;
use pacer::{Pacer, Rule, RuleResult, Verdict, Wait, not_after, not_before};

struct Job {
    deadline: Option<i64>,
}

#[test]
fn not_after_is_ready_before_the_deadline() {
    let rule = not_after(|j: &Job| j.deadline);
    let job = Job {
        deadline: Some(ts("2026-01-01T12:00:00.000Z")),
    };
    assert_eq!(
        rule.check(&job, ts("2026-01-01T10:00:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn not_after_is_ready_exactly_at_the_deadline() {
    let rule = not_after(|j: &Job| j.deadline);
    let deadline = ts("2026-01-01T12:00:00.000Z");
    assert_eq!(
        rule.check(
            &Job {
                deadline: Some(deadline)
            },
            deadline
        ),
        RuleResult::Ready
    );
}

#[test]
fn not_after_blocks_forever_after_the_deadline() {
    let rule = not_after(|j: &Job| j.deadline);
    let deadline = ts("2026-01-01T12:00:00.000Z");
    assert_eq!(
        rule.check(
            &Job {
                deadline: Some(deadline)
            },
            deadline + 1
        ),
        RuleResult::blocked_forever("too late")
    );
}

#[test]
fn not_after_without_deadline_is_ready() {
    let rule = not_after(|j: &Job| j.deadline);
    assert_eq!(
        rule.check(&Job { deadline: None }, i64::MAX),
        RuleResult::Ready
    );
}

#[test]
fn not_after_supports_name_and_reason_overrides() {
    let rule = not_after(|j: &Job| j.deadline)
        .named("offer-expiry")
        .with_reason_fn(|_, now, deadline| format!("{}ms late", now - deadline));
    assert_eq!(rule.name(), "offer-expiry");
    assert_eq!(
        rule.check(
            &Job {
                deadline: Some(100)
            },
            150
        ),
        RuleResult::blocked_forever("50ms late")
    );
}

#[test]
fn not_after_never_overrides_a_timed_block_in_the_pacer() {
    struct Ctx {
        start: i64,
        deadline: i64,
    }
    let pacer: Pacer<Ctx> = Pacer::new(vec![
        Box::new(not_before(|c: &Ctx| c.start)),
        Box::new(not_after(|c: &Ctx| Some(c.deadline))),
    ]);
    // Start in the future but deadline already passed: never.
    let verdict = pacer.check(
        &Ctx {
            start: 200,
            deadline: 50,
        },
        100,
    );
    assert_eq!(verdict.wait(), Some(Wait::Never));
    assert_eq!(verdict.next_at(100), None);
    assert_eq!(verdict.blocks().len(), 2);

    // Inside [start, deadline]: ready.
    assert_eq!(
        pacer.check(
            &Ctx {
                start: 50,
                deadline: 200
            },
            100
        ),
        Verdict::Ready
    );
}
