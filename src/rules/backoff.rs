use super::{Reason, ScopeFn, builder_methods};
use crate::types::{Millis, Rule, RuleResult};

type AttemptsFn<C> = dyn Fn(&C, &str) -> u32 + Send + Sync;
type LastFn<C> = dyn Fn(&C, &str) -> Option<Millis> + Send + Sync;
type ReasonFn<C> = dyn Fn(&C, Millis, Millis, &str, u32, Millis) -> String + Send + Sync;

/// Progressively longer waits between attempts. Build with [`backoff`].
pub struct Backoff<C: ?Sized> {
    name: String,
    scope: ScopeFn<C>,
    base_ms: Millis,
    multiplier: f64,
    max_ms: Option<Millis>,
    get_attempts: Box<AttemptsFn<C>>,
    get_last_attempt_time: Box<LastFn<C>>,
    reason: Reason<ReasonFn<C>>,
}

/// Blocked until `last_attempt_time + delay(attempts)` with
/// `delay = min(base_ms * multiplier^(attempts - 1), max_ms)`.
///
/// Zero attempts or no last attempt time → ready. Defaults: multiplier `2.0`,
/// no cap. Panics if `base_ms <= 0`, `multiplier` is not a positive finite
/// number, or `max_ms <= 0`. Default name `"backoff"`, default reason
/// `"backoff active (attempt <n>, delay <round(delay/1000)>s)"`. Reason closure
/// args: `(ctx, now, retry_at, scope, attempts, delay_ms)`.
pub fn backoff<C, S, A, L>(
    scope: S,
    base_ms: Millis,
    get_attempts: A,
    get_last_attempt_time: L,
) -> Backoff<C>
where
    C: ?Sized,
    S: Fn(&C) -> String + Send + Sync + 'static,
    A: Fn(&C, &str) -> u32 + Send + Sync + 'static,
    L: Fn(&C, &str) -> Option<Millis> + Send + Sync + 'static,
{
    assert!(base_ms > 0, "base_ms must be positive");
    Backoff {
        name: "backoff".to_owned(),
        scope: Box::new(scope),
        base_ms,
        multiplier: 2.0,
        max_ms: None,
        get_attempts: Box::new(get_attempts),
        get_last_attempt_time: Box::new(get_last_attempt_time),
        reason: Reason::Default,
    }
}

impl<C: ?Sized> Backoff<C> {
    builder_methods!(&C, Millis, Millis, &str, u32, Millis);

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
    pub fn max_ms(mut self, max_ms: Millis) -> Self {
        assert!(max_ms > 0, "max_ms must be positive");
        self.max_ms = Some(max_ms);
        self
    }
}

impl<C: ?Sized> Rule<C> for Backoff<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, ctx: &C, now: Millis) -> RuleResult {
        let scope = (self.scope)(ctx);
        let attempts = (self.get_attempts)(ctx, &scope);
        if attempts == 0 {
            return RuleResult::Ready;
        }
        let Some(last) = (self.get_last_attempt_time)(ctx, &scope) else {
            return RuleResult::Ready;
        };

        let raw = self.base_ms as f64 * self.multiplier.powf(f64::from(attempts - 1));
        let delay = self.max_ms.map_or(raw, |max| raw.min(max as f64));
        // Like JS `new Date(ms)`: fractional milliseconds truncate; `as` saturates.
        let delay_ms = delay.trunc() as Millis;
        let retry_at = last.saturating_add(delay_ms);
        if now >= retry_at {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(
            retry_at,
            self.reason.resolve(
                |f| f(ctx, now, retry_at, &scope, attempts, delay_ms),
                || {
                    // JS Math.round: half rounds towards +infinity.
                    let secs = (delay / 1000.0 + 0.5).floor();
                    format!("backoff active (attempt {attempts}, delay {secs}s)")
                },
            ),
        )
    }
}
