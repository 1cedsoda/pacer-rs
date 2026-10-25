#![cfg(feature = "tz")]

mod common;
#[allow(unused_imports)]
use common::{ms, span};

use common::ts;
use pacer::tz::{LocalTime, TimeZone, Weekday, zone};
use pacer::{Rule, RuleResult, Wait, ZonedWindow, zoned_window};

fn hm(h: i8, m: i8) -> LocalTime {
    LocalTime::new(h, m, 0, 0).unwrap()
}

#[allow(non_snake_case)]
fn Berlin() -> TimeZone {
    zone("Europe/Berlin")
}

fn business_hours() -> ZonedWindow {
    zoned_window(Berlin(), hm(9, 0), hm(20, 0))
}

fn check(rule: &ZonedWindow, now: &str) -> RuleResult {
    Rule::<()>::check(rule, &(), ts(now))
}

#[test]
fn open_inside_local_hours_in_winter_and_summer() {
    let rule = business_hours();
    // Winter (CET, UTC+1): 09:00 local = 08:00Z.
    assert_eq!(check(&rule, "2026-01-15T08:00:00.000Z"), RuleResult::Ready);
    assert_eq!(check(&rule, "2026-01-15T18:59:59.999Z"), RuleResult::Ready);
    // Summer (CEST, UTC+2): 09:00 local = 07:00Z.
    assert_eq!(check(&rule, "2026-07-15T07:00:00.000Z"), RuleResult::Ready);
    assert_eq!(check(&rule, "2026-07-15T17:59:59.999Z"), RuleResult::Ready);
}

#[test]
fn closed_before_open_blocks_until_same_day_open() {
    let rule = business_hours();
    assert_eq!(
        check(&rule, "2026-01-15T07:59:59.999Z"),
        RuleResult::blocked_until(ts("2026-01-15T08:00:00.000Z"), "outside zoned window")
    );
}

#[test]
fn closed_after_end_blocks_until_next_day_open() {
    let rule = business_hours();
    // 20:00 local (end is exclusive) → next day 09:00 local.
    assert_eq!(
        check(&rule, "2026-01-15T19:00:00.000Z"),
        RuleResult::blocked_until(ts("2026-01-16T08:00:00.000Z"), "outside zoned window")
    );
}

#[test]
fn dst_fall_back_2026_10_25_berlin() {
    // CEST → CET at 2026-10-25 03:00 CEST (01:00Z).
    let rule = business_hours();

    // Saturday 24th, 21:00 CEST: next open is Sunday 09:00 CET = 08:00Z,
    // not 07:00Z (which a naive "+24h" from Saturday's open would give).
    assert_eq!(
        check(&rule, "2026-10-24T19:00:00.000Z"),
        RuleResult::blocked_until(ts("2026-10-25T08:00:00.000Z"), "outside zoned window")
    );
    // Saturday is still on summer time: open 07:00Z .. 18:00Z.
    assert_eq!(check(&rule, "2026-10-24T07:00:00.000Z"), RuleResult::Ready);
    assert!(!check(&rule, "2026-10-24T18:00:00.000Z").is_ready());

    // Sunday 25th 08:30 CET (07:30Z) is still closed.
    assert_eq!(
        check(&rule, "2026-10-25T07:30:00.000Z"),
        RuleResult::blocked_until(ts("2026-10-25T08:00:00.000Z"), "outside zoned window")
    );
    // Sunday open 08:00Z .. 19:00Z.
    assert_eq!(check(&rule, "2026-10-25T08:00:00.000Z"), RuleResult::Ready);
    assert_eq!(check(&rule, "2026-10-25T18:30:00.000Z"), RuleResult::Ready);
    assert!(!check(&rule, "2026-10-25T19:00:00.000Z").is_ready());

    // Exactly at the switch instant.
    assert_eq!(
        rule.next_open(ts("2026-10-25T01:00:00.000Z")),
        Some(ts("2026-10-25T08:00:00.000Z"))
    );
}

#[test]
fn dst_ambiguous_start_uses_the_earlier_occurrence() {
    // 02:30 happens twice on 2026-10-25 in Berlin: 00:30Z (CEST) and 01:30Z (CET).
    let rule = zoned_window(Berlin(), hm(2, 30), hm(4, 0));
    assert_eq!(
        rule.next_open(ts("2026-10-25T00:00:00.000Z")),
        Some(ts("2026-10-25T00:30:00.000Z"))
    );
    assert!(rule.is_open(ts("2026-10-25T00:30:00.000Z")));
    assert!(rule.is_open(ts("2026-10-25T02:59:59.999Z"))); // 03:59:59 CET
    assert!(!rule.is_open(ts("2026-10-25T03:00:00.000Z"))); // 04:00 CET
}

#[test]
fn dst_spring_forward_gap_opens_at_the_transition() {
    // 2026-03-29: 02:00 CET jumps to 03:00 CEST at 01:00Z; 02:30 does not exist.
    let rule = zoned_window(Berlin(), hm(2, 30), hm(5, 0));
    assert_eq!(
        rule.next_open(ts("2026-03-29T00:00:00.000Z")),
        Some(ts("2026-03-29T01:00:00.000Z"))
    );
    assert!(!rule.is_open(ts("2026-03-29T00:59:59.999Z")));
    assert!(rule.is_open(ts("2026-03-29T01:00:00.000Z")));
    // 05:00 CEST = 03:00Z.
    assert!(!rule.is_open(ts("2026-03-29T03:00:00.000Z")));
    // The day after, 02:30 CEST = 00:30Z.
    assert_eq!(
        rule.next_open(ts("2026-03-29T03:00:00.000Z")),
        Some(ts("2026-03-30T00:30:00.000Z"))
    );
}

#[test]
fn weekdays_skip_to_next_allowed_day() {
    use Weekday::*;
    let rule = business_hours().on_weekdays([Monday, Tuesday, Wednesday, Thursday, Friday]);
    // Saturday 2026-10-24 noon CEST → Monday 2026-10-26 09:00 CET = 08:00Z.
    assert_eq!(
        check(&rule, "2026-10-24T10:00:00.000Z"),
        RuleResult::blocked_until(ts("2026-10-26T08:00:00.000Z"), "outside zoned window")
    );
    // Friday 2026-10-23 19:59 CEST is open.
    assert_eq!(check(&rule, "2026-10-23T17:59:00.000Z"), RuleResult::Ready);
    // Friday after close → Monday.
    assert_eq!(
        rule.next_open(ts("2026-10-23T18:00:00.000Z")),
        Some(ts("2026-10-26T08:00:00.000Z"))
    );
}

#[test]
fn weekday_is_evaluated_in_local_time_not_utc() {
    // Monday 2026-01-12 00:30 CET is Sunday 23:30Z: Monday-only window from
    // 00:00 local must be open even though the UTC weekday is Sunday.
    let rule = zoned_window(Berlin(), hm(0, 0), hm(1, 0)).on_weekdays([Weekday::Monday]);
    assert!(rule.is_open(ts("2026-01-11T23:30:00.000Z")));
}

#[test]
fn overnight_window_spans_midnight() {
    use Weekday::*;
    // Friday-night window 22:00–06:00, opening day Friday.
    let rule = zoned_window(Berlin(), hm(22, 0), hm(6, 0)).on_weekdays([Friday]);
    // Saturday 2026-01-17 03:00 CET (02:00Z) belongs to Friday's window.
    assert!(rule.is_open(ts("2026-01-17T02:00:00.000Z")));
    // Saturday 06:00 CET closes it.
    assert!(!rule.is_open(ts("2026-01-17T05:00:00.000Z")));
    // Next open: Friday 2026-01-23 22:00 CET = 21:00Z.
    assert_eq!(
        rule.next_open(ts("2026-01-17T05:00:00.000Z")),
        Some(ts("2026-01-23T21:00:00.000Z"))
    );
    // Thursday 23:00 local is not open (window opens only on Friday).
    assert!(!rule.is_open(ts("2026-01-15T22:00:00.000Z")));
}

#[test]
fn no_weekdays_blocks_forever() {
    let rule = business_hours()
        .on_weekdays([])
        .named("never-open")
        .with_reason("closed");
    assert_eq!(Rule::<()>::name(&rule), "never-open");
    assert_eq!(
        check(&rule, "2026-01-15T10:00:00.000Z"),
        RuleResult::Blocked {
            wait: Wait::Never,
            reason: "closed".into()
        }
    );
}

#[test]
#[should_panic(expected = "start and end must differ")]
fn equal_start_and_end_panics() {
    let _ = zoned_window(Berlin(), hm(9, 0), hm(9, 0));
}
