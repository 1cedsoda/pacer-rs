mod common;
#[allow(unused_imports)]
use common::{ms, span};

use std::sync::{Arc, Mutex};

use common::ts;
use pacer::{
    Rule, RuleResult, Time, TimeWindow, backoff, concurrency_limit, cooldown, dedupe, fixed_quota,
    fixed_window, min_gap, not_before, sliding_quota,
};

fn blocked(at: &str, reason: &str) -> RuleResult {
    RuleResult::blocked_until(ts(at), reason)
}

// ─── min_gap ─────────────────────────────────────────────────────────────────

struct Email {
    email: String,
}

#[test]
fn min_gap_blocks_until_the_minimum_gap_has_elapsed() {
    let rule = min_gap(
        |c: &Email| format!("email:{}", c.email),
        span(60_000),
        |_: &Email, _: &str| Some(ts("2026-01-01T10:00:30.000Z")),
    );
    let ctx = Email {
        email: "a@example.com".into(),
    };
    assert_eq!(
        rule.check(&ctx, ts("2026-01-01T10:01:00.000Z")),
        blocked("2026-01-01T10:01:30.000Z", "minimum gap has not elapsed")
    );
}

#[test]
fn min_gap_returns_ready_when_there_is_no_last_event() {
    let rule = min_gap(
        |_: &()| "scope".into(),
        span(60_000),
        |_: &(), _: &str| None,
    );
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:01:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn min_gap_accepts_gap_as_a_function_of_context() {
    struct Ctx {
        delay_ms: u64,
    }
    let rule = min_gap(
        |_: &Ctx| "scope".into(),
        |c: &Ctx| span(c.delay_ms),
        |_: &Ctx, _: &str| Some(ts("2026-01-01T10:00:00.000Z")),
    );
    assert_eq!(
        rule.check(&Ctx { delay_ms: 30_000 }, ts("2026-01-01T10:00:20.000Z")),
        blocked("2026-01-01T10:00:30.000Z", "minimum gap has not elapsed")
    );
}

#[test]
fn min_gap_passes_the_scope_and_supports_name_and_reason_overrides() {
    let rule = min_gap(
        |c: &Email| format!("email:{}", c.email),
        span(1_000),
        |_: &Email, scope: &str| {
            assert_eq!(scope, "email:x");
            Some(ms(0))
        },
    )
    .named("email-min-gap")
    .with_reason_fn(|_, now, retry_at, scope, last| {
        format!("{scope}: {now} < {retry_at} (last {last})")
    });
    assert_eq!(rule.name(), "email-min-gap");
    assert_eq!(
        rule.check(&Email { email: "x".into() }, ms(500)),
        RuleResult::blocked_until(
            ms(1_000),
            "email:x: 1970-01-01T00:00:00.500Z < 1970-01-01T00:00:01Z (last 1970-01-01T00:00:00Z)"
        )
    );
}

// ─── fixed_window ────────────────────────────────────────────────────────────

#[test]
fn fixed_window_blocks_until_the_next_window_opens() {
    let rule = fixed_window(
        |_: &(), _| false,
        |_: &(), _| ts("2026-01-01T12:00:00.000Z"),
    );
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        blocked("2026-01-01T12:00:00.000Z", "outside fixed window")
    );
}

#[test]
fn fixed_window_returns_ready_when_window_is_open() {
    let rule = fixed_window(|_: &(), _| true, |_: &(), _| ts("2026-01-01T12:00:00.000Z"));
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
#[should_panic(expected = "next_open must return a future time")]
fn fixed_window_panics_when_next_open_is_not_in_the_future() {
    let rule = fixed_window(|_: &(), _| false, |_: &(), now| now);
    rule.check(&(), ts("2026-01-01T10:00:00.000Z"));
}

// ─── not_before ──────────────────────────────────────────────────────────────

struct Scheduled {
    retry_at: Time,
}

#[test]
fn not_before_blocks_until_the_scheduled_time() {
    let retry_at = ts("2026-01-01T10:05:00.000Z");
    let rule = not_before(|c: &Scheduled| c.retry_at);
    assert_eq!(
        rule.check(&Scheduled { retry_at }, ts("2026-01-01T10:00:00.000Z")),
        RuleResult::blocked_until(retry_at, "not ready before the scheduled time")
    );
}

#[test]
fn not_before_returns_ready_once_the_scheduled_time_has_passed() {
    let retry_at = ts("2026-01-01T10:05:00.000Z");
    let rule = not_before(|c: &Scheduled| c.retry_at);
    assert_eq!(
        rule.check(&Scheduled { retry_at }, ts("2026-01-01T10:06:00.000Z")),
        RuleResult::Ready
    );
    // Inclusive boundary.
    assert_eq!(
        rule.check(&Scheduled { retry_at }, retry_at),
        RuleResult::Ready
    );
}

// ─── fixed_quota ─────────────────────────────────────────────────────────────

#[test]
fn fixed_quota_blocks_until_the_end_of_the_current_fixed_window() {
    let now = ts("2026-01-01T10:30:00.000Z");
    let rule = fixed_quota(
        |c: &Email| format!("email:{}", c.email),
        2,
        |_: &Email, current| TimeWindow::utc_aligned(current, DAY),
        |_: &Email, _: &str, _| 2,
    );
    assert_eq!(
        rule.check(
            &Email {
                email: "a@example.com".into()
            },
            now
        ),
        blocked("2026-01-02T00:00:00.000Z", "fixed quota exhausted (2/2)")
    );
}

#[test]
fn fixed_quota_returns_ready_when_count_is_below_the_limit() {
    let now = ts("2026-01-01T10:30:00.000Z");
    let rule = fixed_quota(
        |_: &()| "scope".into(),
        3,
        |_: &(), current| TimeWindow::utc_aligned(current, DAY),
        |_: &(), _: &str, _| 2,
    );
    assert_eq!(rule.check(&(), now), RuleResult::Ready);
}

#[test]
#[should_panic(expected = "window.end must be after window.start")]
fn fixed_quota_panics_on_empty_window() {
    let rule = fixed_quota(
        |_: &()| "scope".into(),
        1,
        |_: &(), now| TimeWindow::new(now, now),
        |_: &(), _: &str, _| 0,
    );
    rule.check(&(), ms(0));
}

#[test]
#[should_panic(expected = "limit must be positive")]
fn fixed_quota_panics_on_zero_limit() {
    let _ = fixed_quota(
        |_: &()| "scope".into(),
        0,
        |_: &(), now| TimeWindow::new(now, now + span(1)),
        |_: &(), _: &str, _| 0,
    );
}

// ─── sliding_quota ───────────────────────────────────────────────────────────

#[test]
fn sliding_quota_blocks_until_enough_events_age_out_of_the_rolling_window() {
    let now = ts("2026-01-01T10:50:00.000Z");
    let rule = sliding_quota(
        |_: &()| "scope".into(),
        3,
        span(60 * 60_000),
        |_: &(), _: &str, _, _| {
            vec![
                ts("2026-01-01T10:00:00.000Z"),
                ts("2026-01-01T10:10:00.000Z"),
                ts("2026-01-01T10:20:00.000Z"),
                ts("2026-01-01T10:30:00.000Z"),
            ]
        },
    );
    assert_eq!(
        rule.check(&(), now),
        blocked("2026-01-01T11:10:00.000Z", "sliding quota exhausted (4/3)")
    );
}

#[test]
fn sliding_quota_returns_ready_when_fewer_than_the_limit_fall_inside_the_rolling_window() {
    let now = ts("2026-01-01T10:50:00.000Z");
    let rule = sliding_quota(
        |_: &()| "scope".into(),
        3,
        span(60 * 60_000),
        |_: &(), _: &str, _, _| {
            vec![
                ts("2026-01-01T09:40:00.000Z"),
                ts("2026-01-01T10:10:00.000Z"),
            ]
        },
    );
    assert_eq!(rule.check(&(), now), RuleResult::Ready);
}

#[test]
fn sliding_quota_ignores_out_of_window_events_and_sorts() {
    let now = ts("2026-01-01T10:50:00.000Z");
    let rule = sliding_quota(
        |_: &()| "scope".into(),
        2,
        span(60 * 60_000),
        |_: &(), _: &str, since, until| {
            assert_eq!(since, ts("2026-01-01T09:50:00.000Z"));
            assert_eq!(until, ts("2026-01-01T10:50:00.000Z"));
            vec![
                ts("2026-01-01T10:40:00.000Z"),
                ts("2026-01-01T09:50:00.000Z"), // exactly `since`: excluded
                ts("2026-01-01T10:55:00.000Z"), // future: excluded
                ts("2026-01-01T10:20:00.000Z"),
            ]
        },
    );
    assert_eq!(
        rule.check(&(), now),
        blocked("2026-01-01T11:20:00.000Z", "sliding quota exhausted (2/2)")
    );
}

// ─── cooldown ────────────────────────────────────────────────────────────────

#[test]
fn cooldown_blocks_until_the_cooldown_expires() {
    let rule = cooldown(
        |_: &()| "scope".into(),
        span(5 * 60_000),
        |_: &(), _: &str| Some(ts("2026-01-01T10:00:00.000Z")),
    );
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:02:00.000Z")),
        blocked("2026-01-01T10:05:00.000Z", "cooldown is still active")
    );
}

#[test]
fn cooldown_returns_ready_when_no_failure_has_occurred() {
    let rule = cooldown(
        |_: &()| "scope".into(),
        span(5 * 60_000),
        |_: &(), _: &str| None,
    );
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn cooldown_accepts_cooldown_as_a_function_of_context() {
    struct Ctx {
        attempts: u32,
    }
    let rule = cooldown(
        |_: &Ctx| "scope".into(),
        |c: &Ctx| span(1_000 * 2_u64.pow(c.attempts)),
        |_: &Ctx, _: &str| Some(ts("2026-01-01T10:00:00.000Z")),
    );
    // attempts=3 → cooldown = 1000 * 8 = 8000ms
    assert_eq!(
        rule.check(&Ctx { attempts: 3 }, ts("2026-01-01T10:00:05.000Z")),
        blocked("2026-01-01T10:00:08.000Z", "cooldown is still active")
    );
}

// ─── concurrency_limit ───────────────────────────────────────────────────────

#[test]
fn concurrency_limit_blocks_with_a_short_retry_when_the_limit_is_reached() {
    let now = ts("2026-01-01T10:00:00.000Z");
    let rule =
        concurrency_limit(|_: &()| "scope".into(), 1, |_: &(), _: &str| 1).retry_after(span(2_500));
    assert_eq!(
        rule.check(&(), now),
        RuleResult::blocked_until(now + span(2_500), "1/1 inflight")
    );
}

#[test]
fn concurrency_limit_returns_ready_when_inflight_count_is_below_the_limit() {
    let rule = concurrency_limit(|_: &()| "scope".into(), 2, |_: &(), _: &str| 1);
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn concurrency_limit_defaults_to_five_seconds_and_supports_retry_fn() {
    let now = ts("2026-01-01T10:00:00.000Z");
    let default = concurrency_limit(|_: &()| "scope".into(), 1, |_: &(), _: &str| 3);
    assert_eq!(
        default.check(&(), now),
        RuleResult::blocked_until(now + span(5_000), "3/1 inflight")
    );

    let dynamic = concurrency_limit(|_: &()| "scope".into(), 1, |_: &(), _: &str| 3)
        .retry_after_fn(|_, _, _, active| span(active * 1_000));
    assert_eq!(
        dynamic.check(&(), now),
        RuleResult::blocked_until(now + span(3_000), "3/1 inflight")
    );
}

// ─── backoff ─────────────────────────────────────────────────────────────────

fn backoff_rule(base_ms: u64, attempts: u32, last: Option<&'static str>) -> pacer::Backoff<()> {
    backoff(
        |_: &()| "scope".into(),
        span(base_ms),
        move |_: &(), _: &str| attempts,
        move |_: &(), _: &str| last.map(ts),
    )
}

#[test]
fn backoff_returns_ready_when_no_attempts_have_been_made() {
    let rule = backoff_rule(1_000, 0, Some("2026-01-01T10:00:00.000Z"));
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn backoff_returns_ready_when_no_last_attempt_time_is_recorded() {
    let rule = backoff_rule(1_000, 3, None);
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn backoff_computes_exponential_delay_for_first_attempt() {
    // attempt=1: delay = 10_000 * 2^0 = 10_000ms
    let rule = backoff_rule(10_000, 1, Some("2026-01-01T10:00:00.000Z"));
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:05.000Z")),
        blocked(
            "2026-01-01T10:00:10.000Z",
            "backoff active (attempt 1, delay 10s)"
        )
    );
}

#[test]
fn backoff_doubles_the_delay_on_each_successive_attempt() {
    // attempt=3: delay = 10_000 * 2^2 = 40_000ms
    let rule = backoff_rule(10_000, 3, Some("2026-01-01T10:00:00.000Z"));
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:30.000Z")),
        blocked(
            "2026-01-01T10:00:40.000Z",
            "backoff active (attempt 3, delay 40s)"
        )
    );
}

#[test]
fn backoff_caps_delay_at_max_ms() {
    // uncapped would be 10_000 * 2^4 = 160_000ms; capped at 30_000ms
    let rule = backoff_rule(10_000, 5, Some("2026-01-01T10:00:00.000Z")).max(span(30_000));
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:20.000Z")),
        blocked(
            "2026-01-01T10:00:30.000Z",
            "backoff active (attempt 5, delay 30s)"
        )
    );
}

#[test]
fn backoff_supports_multiplier_one_for_flat_backoff() {
    // flat: always 5_000ms regardless of attempts
    let rule = backoff_rule(5_000, 4, Some("2026-01-01T10:00:00.000Z")).multiplier(1.0);
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:03.000Z")),
        blocked(
            "2026-01-01T10:00:05.000Z",
            "backoff active (attempt 4, delay 5s)"
        )
    );
}

#[test]
fn backoff_returns_ready_once_the_backoff_window_has_elapsed() {
    // attempt=2: delay = 10_000 * 2^1 = 20_000ms
    let rule = backoff_rule(10_000, 2, Some("2026-01-01T10:00:00.000Z"));
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:25.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn backoff_rounds_reason_seconds_like_js_math_round() {
    // 1_500ms → Math.round(1.5) = 2
    let rule = backoff_rule(1_500, 1, Some("2026-01-01T10:00:00.000Z"));
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        blocked(
            "2026-01-01T10:00:01.500Z",
            "backoff active (attempt 1, delay 2s)"
        )
    );
}

#[test]
fn backoff_saturates_huge_delays_instead_of_overflowing() {
    let rule = backoff_rule(1_000, u32::MAX, Some("2026-01-01T10:00:00.000Z"));
    match rule.check(&(), ts("2026-01-01T10:00:00.000Z")) {
        RuleResult::Blocked { wait, .. } => assert_eq!(wait, pacer::Wait::At(ms(i64::MAX))),
        RuleResult::Ready => panic!("expected blocked"),
    }
}

// ─── dedupe ──────────────────────────────────────────────────────────────────

#[test]
fn dedupe_returns_ready_when_the_item_has_not_been_processed() {
    let rule = dedupe(|_: &()| "scope".into(), |_: &(), _: &str| None);
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        RuleResult::Ready
    );
}

#[test]
fn dedupe_returns_permanently_blocked_when_already_processed() {
    let completed_at = ts("2026-01-01T09:00:00.000Z");
    let rule = dedupe(
        |_: &()| "scope".into(),
        move |_: &(), _: &str| Some(completed_at),
    );
    assert_eq!(
        rule.check(&(), ts("2026-01-01T10:00:00.000Z")),
        RuleResult::blocked_forever("already processed (2026-01-01T09:00:00Z)")
    );
}

#[test]
fn dedupe_passes_the_scope_to_get_completed_at() {
    struct Item {
        id: String,
    }
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    let rule = dedupe(
        |c: &Item| format!("item:{}", c.id),
        move |_: &Item, scope: &str| {
            sink.lock().unwrap().push(scope.to_owned());
            None
        },
    );
    rule.check(&Item { id: "abc".into() }, ts("2026-01-01T10:00:00.000Z"));
    assert_eq!(*seen.lock().unwrap(), ["item:abc"]);
}

// ─── UTC windows and display ─────────────────────────────────────────────────

const HOUR: std::time::Duration = std::time::Duration::from_secs(3600);
const DAY: std::time::Duration = std::time::Duration::from_secs(86_400);

fn base() -> Time {
    ts("2026-01-15T14:37:22.456Z") // 14:37 UTC
}

#[test]
fn utc_hours_and_days_are_aligned_windows() {
    let hour = TimeWindow::utc_aligned(base(), HOUR);
    assert_eq!(
        (hour.start, hour.end),
        (ts("2026-01-15T14:00:00Z"), ts("2026-01-15T15:00:00Z"))
    );
    let day = TimeWindow::utc_aligned(base(), DAY);
    assert_eq!(
        (day.start, day.end),
        (ts("2026-01-15T00:00:00Z"), ts("2026-01-16T00:00:00Z"))
    );
    let midnight = ts("2026-01-15T00:00:00Z");
    assert_eq!(
        TimeWindow::utc_aligned(midnight, HOUR).end,
        ts("2026-01-15T01:00:00Z")
    );
    assert_eq!(
        TimeWindow::utc_aligned(midnight, DAY).end,
        ts("2026-01-16T00:00:00Z")
    );
    assert!(day.contains(base()) && !day.contains(day.end));
    assert_eq!(day.len(), DAY);
}

#[test]
fn windows_handle_times_before_the_epoch() {
    let t = ts("1969-12-31T23:30:00Z");
    assert_eq!(t, ms(-30 * 60_000));
    assert_eq!(
        TimeWindow::utc_aligned(t, DAY).start,
        ts("1969-12-31T00:00:00Z")
    );
    assert_eq!(
        TimeWindow::utc_aligned(t, HOUR).start,
        ts("1969-12-31T23:00:00Z")
    );
    assert_eq!(TimeWindow::utc_aligned(t, HOUR).end, Time::UNIX_EPOCH);
}

#[test]
fn times_display_as_rfc3339() {
    assert_eq!(Time::UNIX_EPOCH.to_string(), "1970-01-01T00:00:00Z");
    assert_eq!(base().to_string(), "2026-01-15T14:37:22.456Z");
    assert_eq!(
        ts("2024-02-29T23:59:59.999Z").to_string(),
        "2024-02-29T23:59:59.999Z"
    );
}
