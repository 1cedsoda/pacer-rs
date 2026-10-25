use std::time::Duration;

use super::{Reason, ScopeFn, builder_methods};
use crate::time::{Time, span_micros};
use crate::types::{Rule, RuleResult, SpanFn};

type TatFn<C> = dyn Fn(&C, &str) -> Option<Time> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Time, Time, &str) -> String + Send + Sync;

/// Token bucket via the generic cell rate algorithm. Build with [`gcra`].
pub struct Gcra<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    emission_interval: Box<dyn SpanFn<C>>,
    burst: u32,
    get_tat: Box<TatFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Token bucket without a refill task (generic cell rate algorithm, GCRA).
///
/// A full bucket allows `burst` cells at once, then one cell every
/// `emission_interval`. The only state per scope is its theoretical arrival
/// time (TAT), read with `get_tat`; `None` means never seen, a full bucket.
///
/// With `tau = emission_interval * (burst - 1)` the rule is ready iff
/// `tat.unwrap_or(now) - tau <= now`, otherwise blocked until `tat - tau`.
///
/// The rule only checks. After an allowed cell, store
/// [`gcra_spend`]`(tat, now, emission_interval)` as the new TAT, atomically
/// together with the check (e.g. under the same lock): pacer itself is pure.
/// A scope whose stored `tat <= now` is a full bucket and may be dropped from
/// the store.
///
/// `emission_interval` is a constant [`Duration`] or `Fn(&C) -> Duration`; it
/// must be positive (panics at check time otherwise). Panics if `burst == 0`.
/// Default name `"gcra"`, default reason `"rate limited (retry in <s>s)"` with
/// the wait rounded up to whole seconds. Reason closure args:
/// `(ctx, now, retry_at, scope)`.
pub fn gcra<C, S, I, T>(scope: S, emission_interval: I, burst: u32, get_tat: T) -> Gcra<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    I: SpanFn<C> + 'static,
    T: Fn(&C, &str) -> Option<Time> + Send + Sync + 'static,
{
    assert!(burst > 0, "burst must be positive");
    Gcra {
        name: "gcra".to_owned(),
        scope: Box::new(scope),
        emission_interval: Box::new(emission_interval),
        burst,
        get_tat: Box::new(get_tat),
        reason: Reason::Default,
    }
}

/// The new theoretical arrival time after one allowed cell:
/// `max(tat.unwrap_or(now), now) + emission_interval`.
///
/// Call it only after [`gcra`] returned ready, and store the result atomically
/// together with that check. Panics if `emission_interval` is zero.
pub fn gcra_spend(tat: Option<Time>, now: Time, emission_interval: Duration) -> Time {
    assert_positive_interval(emission_interval);
    tat.unwrap_or(now)
        .max(now)
        .saturating_add(emission_interval)
}

fn assert_positive_interval(interval: Duration) -> Duration {
    assert!(
        !interval.is_zero(),
        "emission_interval must be positive, got {interval:?}"
    );
    interval
}

impl<C: ?Sized> Gcra<C> {
    builder_methods!(&C, Time, Time, &str);
}

impl<C: ?Sized> Rule<C> for Gcra<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        let interval = assert_positive_interval(self.emission_interval.span(ctx));
        let scope = (self.scope)(ctx);
        let Some(tat) = (self.get_tat)(ctx, &scope) else {
            return RuleResult::Ready;
        };
        let tau = interval.saturating_mul(self.burst - 1);
        let retry_at = tat.saturating_sub(tau);
        if retry_at <= now {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope),
                || {
                    // `retry_at > now`, so this rounds the wait up to whole seconds.
                    let wait = span_micros(retry_at.since(now)).unwrap_or(i64::MAX);
                    let secs = (wait - 1) / 1_000_000 + 1;
                    format!("rate limited (retry in {secs}s)")
                },
            ),
        )
    }
}
