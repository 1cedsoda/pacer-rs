use super::{Reason, ScopeFn, builder_methods};
use std::time::Duration;

use crate::time::Time;
use crate::types::{Rule, RuleResult};

type AttemptsFn<C> = dyn Fn(&C, &str) -> u32 + Send + Sync;
type LastFn<C> = dyn Fn(&C, &str) -> Option<Time> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Time, Time, &str, u32, Duration) -> String + Send + Sync;

/// Progressively longer waits between attempts. Build with [`backoff`].
pub struct Backoff<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    base: Duration,
    multiplier: f64,
    max: Option<Duration>,
    get_attempts: Box<AttemptsFn<C>>,
    get_last_attempt_time: Box<LastFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Blocked until `last_attempt_time + delay(attempts)` with
/// `delay = min(base * multiplier^(attempts - 1), max)`.
///
/// Zero attempts or no last attempt time → ready. Defaults: multiplier `2.0`,
/// no cap. The delay is computed in microseconds and truncated. Panics if
/// `base` is zero, `multiplier` is not a positive finite number, or `max` is
/// zero. Default name `"backoff"`, default reason
/// `"backoff active (attempt <n>, delay <round(delay/1000)>s)"`. Reason closure
/// args: `(ctx, now, retry_at, scope, attempts, delay)`.
pub fn backoff<C, S, A, L>(
    scope: S,
    base: Duration,
    get_attempts: A,
    get_last_attempt_time: L,
) -> Backoff<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    A: Fn(&C, &str) -> u32 + Send + Sync + 'static,
    L: Fn(&C, &str) -> Option<Time> + Send + Sync + 'static,
{
    assert!(!base.is_zero(), "base must be positive");
    Backoff {
        name: "backoff".to_owned(),
        scope: Box::new(scope),
        base,
        multiplier: 2.0,
        max: None,
        get_attempts: Box::new(get_attempts),
        get_last_attempt_time: Box::new(get_last_attempt_time),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> Backoff<C> {
    builder_methods!(&C, Time, Time, &str, u32, Duration);

    /// Growth factor per attempt (default `2.0`; `1.0` = flat).
    pub fn multiplier(mut self, multiplier: f64) -> Self {
        assert!(
            multiplier.is_finite() && multiplier > 0.0,
            "multiplier must be a positive finite number"
        );
        self.multiplier = multiplier;
        self
    }

    /// Upper bound on the computed delay.
    pub fn max(mut self, max: Duration) -> Self {
        assert!(!max.is_zero(), "max must be positive");
        self.max = Some(max);
        self
    }
}

impl<C: ?Sized> Rule<C> for Backoff<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Time) -> RuleResult {
        let scope = (self.scope)(ctx);
        let attempts = (self.get_attempts)(ctx, &scope);
        if attempts == 0 {
            return RuleResult::Ready;
        }
        let Some(last) = (self.get_last_attempt_time)(ctx, &scope) else {
            return RuleResult::Ready;
        };

        let micros = |d: Duration| d.as_micros() as f64;
        let raw = micros(self.base) * self.multiplier.powf(f64::from(attempts - 1));
        let delay_us = self.max.map_or(raw, |max| raw.min(micros(max)));
        // Fractional microseconds truncate; `as` saturates.
        let delay = Duration::from_micros(delay_us.trunc() as u64);
        let retry_at = last.saturating_add(delay);
        if now >= retry_at {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope, attempts, delay),
                || {
                    // JS Math.round: half rounds towards +infinity.
                    let secs = (delay_us / 1_000_000.0 + 0.5).floor();
                    format!("backoff active (attempt {attempts}, delay {secs}s)")
                },
            ),
        )
    }
}
