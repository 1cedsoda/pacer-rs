#![allow(dead_code)]

use pacer::{Millis, RuleResult, rule_fn};

/// Parse `YYYY-MM-DDTHH:MM:SS.mmmZ` into epoch milliseconds.
pub fn ts(iso: &str) -> Millis {
    let n = |range: std::ops::Range<usize>| iso[range].parse::<i64>().unwrap();
    assert!(iso.len() == 24 && iso.ends_with('Z'), "bad timestamp {iso}");
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    let (h, mi, s, ms) = (n(11..13), n(14..16), n(17..19), n(20..23));
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 24 + h) * 60 + mi) * 60_000 + s * 1_000 + ms
}

pub fn ready_rule(
    name: &str,
) -> pacer::FnRule<impl Fn(&(), Millis) -> RuleResult + Send + Sync + use<>> {
    rule_fn(name, |_: &(), _| RuleResult::Ready)
}

pub fn blocked_rule(
    name: &str,
    retry_at: &str,
    reason: &str,
) -> pacer::FnRule<impl Fn(&(), Millis) -> RuleResult + Send + Sync + use<>> {
    let at = ts(retry_at);
    let reason = reason.to_owned();
    rule_fn(name, move |_: &(), _| {
        RuleResult::blocked_until(at, reason.clone())
    })
}

pub fn never_rule(
    name: &str,
    reason: &str,
) -> pacer::FnRule<impl Fn(&(), Millis) -> RuleResult + Send + Sync + use<>> {
    let reason = reason.to_owned();
    rule_fn(name, move |_: &(), _| {
        RuleResult::blocked_forever(reason.clone())
    })
}
