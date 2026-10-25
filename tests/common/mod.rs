#![allow(dead_code)]

use pacer::{RuleResult, Time, rule_fn};

/// Parse an RFC 3339 time.
pub fn ts(iso: &str) -> Time {
    iso.parse().unwrap()
}

/// Epoch milliseconds as a time.
pub fn ms(millis: i64) -> Time {
    Time::from_millis(millis)
}

/// Timeeconds as a span.
pub fn span(millis: u64) -> std::time::Duration {
    std::time::Duration::from_millis(millis)
}

pub fn ready_rule(
    name: &str,
) -> pacer::FnRule<impl Fn(&(), Time) -> RuleResult + Send + Sync + use<>> {
    rule_fn(name, |_: &(), _| RuleResult::Ready)
}

pub fn blocked_rule(
    name: &str,
    retry_at: &str,
    reason: &str,
) -> pacer::FnRule<impl Fn(&(), Time) -> RuleResult + Send + Sync + use<>> {
    let at = ts(retry_at);
    let reason = reason.to_owned();
    rule_fn(name, move |_: &(), _| {
        RuleResult::blocked_until(at, reason.clone())
    })
}

pub fn never_rule(
    name: &str,
    reason: &str,
) -> pacer::FnRule<impl Fn(&(), Time) -> RuleResult + Send + Sync + use<>> {
    let reason = reason.to_owned();
    rule_fn(name, move |_: &(), _| {
        RuleResult::blocked_forever(reason.clone())
    })
}
