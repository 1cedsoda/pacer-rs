mod common;
#[allow(unused_imports)]
use common::{ms, span};

use std::collections::HashMap;

use common::ts;
use pacer::{Rule, RuleResult, Time, Wait, gcra, gcra_spend};

struct Ctx {
    ip: String,
    tat: Option<Time>,
}

fn ctx(tat: Option<Time>) -> Ctx {
    Ctx {
        ip: "10.0.0.1".into(),
        tat,
    }
}

fn rule(interval: std::time::Duration, burst: u32) -> pacer::Gcra<Ctx> {
    gcra(
        |c: &Ctx| format!("ip:{}", c.ip),
        interval,
        burst,
        |c: &Ctx, _: &str| c.tat,
    )
}

/// Check, and on ready spend one cell. Returns whether the cell passed.
fn take(
    rule: &pacer::Gcra<Ctx>,
    state: &mut Ctx,
    now: Time,
    interval: std::time::Duration,
) -> RuleResult {
    let result = rule.check(state, now);
    if result.is_ready() {
        state.tat = Some(gcra_spend(state.tat, now, interval));
    }
    result
}

#[test]
fn gcra_allows_a_burst_then_blocks_with_the_exact_retry_time() {
    let rule = rule(span(1_000), 3);
    let mut state = ctx(None);
    let now = ts("2026-01-01T10:00:00.000Z");
    for _ in 0..3 {
        assert_eq!(take(&rule, &mut state, now, span(1_000)), RuleResult::Ready);
    }
    assert_eq!(state.tat, Some(now + span(3_000)));
    assert_eq!(
        take(&rule, &mut state, now, span(1_000)),
        RuleResult::blocked_until(now + span(1_000), "rate limited (retry in 1s)")
    );
    // Still blocked one millisecond before, ready exactly at retry_at.
    assert!(!rule.check(&state, now + span(999)).is_ready());
    assert!(rule.check(&state, now + span(1_000)).is_ready());
}

#[test]
fn gcra_refills_one_cell_per_interval() {
    let rule = rule(span(500), 2);
    let mut state = ctx(None);
    let now = ts("2026-01-01T10:00:00.000Z");
    assert!(take(&rule, &mut state, now, span(500)).is_ready());
    assert!(take(&rule, &mut state, now, span(500)).is_ready());
    assert!(!take(&rule, &mut state, now, span(500)).is_ready());
    // One interval later exactly one cell is back.
    assert!(take(&rule, &mut state, now + span(500), span(500)).is_ready());
    assert_eq!(
        rule.check(&state, now + span(500)),
        RuleResult::Blocked {
            wait: Wait::At(now + span(1_000)),
            reason: "rate limited (retry in 1s)".into()
        }
    );
}

#[test]
fn gcra_treats_a_never_seen_scope_as_a_full_bucket() {
    let rule = rule(span(60_000), 1);
    assert_eq!(rule.check(&ctx(None), ms(0)), RuleResult::Ready);
    assert_eq!(rule.name(), "gcra");
}

#[test]
fn gcra_treats_a_past_tat_as_a_full_bucket() {
    let rule = rule(span(1_000), 5);
    let now = ts("2026-01-01T10:00:00.000Z");
    let mut state = ctx(Some(now - span(10_000)));
    for _ in 0..5 {
        assert!(take(&rule, &mut state, now, span(1_000)).is_ready());
    }
    assert!(!take(&rule, &mut state, now, span(1_000)).is_ready());
}

#[test]
fn gcra_with_burst_one_spaces_cells_by_the_interval() {
    let rule = rule(span(2_500), 1);
    let mut state = ctx(None);
    let now = ts("2026-01-01T10:00:00.000Z");
    assert!(take(&rule, &mut state, now, span(2_500)).is_ready());
    assert_eq!(
        take(&rule, &mut state, now + span(1_000), span(2_500)),
        RuleResult::blocked_until(now + span(2_500), "rate limited (retry in 2s)")
    );
    assert!(take(&rule, &mut state, now + span(2_500), span(2_500)).is_ready());
}

#[test]
fn gcra_accepts_the_interval_as_a_function_of_context() {
    struct Tenant {
        per_second: u64,
        tat: Option<Time>,
    }
    let rule = gcra(
        |_: &Tenant| "tenant".into(),
        |c: &Tenant| span(1_000 / c.per_second),
        2,
        |c: &Tenant, _: &str| c.tat,
    );
    // 4/s → 250 ms interval, tau = 250: tat 1_000 is ready from 750.
    let ctx = Tenant {
        per_second: 4,
        tat: Some(ms(1_000)),
    };
    assert_eq!(
        rule.check(&ctx, ms(700)),
        RuleResult::blocked_until(ms(750), "rate limited (retry in 1s)")
    );
    assert!(rule.check(&ctx, ms(750)).is_ready());
}

#[test]
fn gcra_reason_closure_and_name_override() {
    let rule =
        rule(span(1_000), 1)
            .named("http")
            .with_reason_fn(|c: &Ctx, now, retry_at, scope| {
                format!("{} {scope} {}", c.ip, retry_at.since(now).as_millis())
            });
    assert_eq!(rule.name(), "http");
    assert_eq!(
        rule.check(&ctx(Some(ms(1_000))), ms(400)),
        RuleResult::blocked_until(ms(1_000), "10.0.0.1 ip:10.0.0.1 600")
    );
    let fixed = self::rule(span(1_000), 1).with_reason("slow down");
    assert_eq!(
        fixed.check(&ctx(Some(ms(1_000))), ms(400)),
        RuleResult::blocked_until(ms(1_000), "slow down")
    );
}

#[test]
fn gcra_spend_advances_from_the_later_of_tat_and_now() {
    assert_eq!(gcra_spend(None, ms(10_000), span(250)), ms(10_250));
    assert_eq!(
        gcra_spend(Some(ms(9_000)), ms(10_000), span(250)),
        ms(10_250)
    );
    assert_eq!(
        gcra_spend(Some(ms(12_000)), ms(10_000), span(250)),
        ms(12_250)
    );
}

#[test]
#[should_panic(expected = "burst must be positive")]
fn gcra_panics_on_zero_burst() {
    let _ = rule(span(1_000), 0);
}

#[test]
#[should_panic(expected = "emission_interval must be positive")]
fn gcra_panics_on_zero_interval() {
    let _ = rule(span(0), 1).check(&ctx(None), ms(0));
}

#[test]
#[should_panic(expected = "emission_interval must be positive")]
fn gcra_spend_panics_on_zero_interval() {
    let _ = gcra_spend(None, ms(0), span(0));
}

/// Deterministic linear congruential generator (Knuth MMIX constants).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

/// Naive token bucket: `credit` is refill time, capacity `burst * interval`,
/// one cell costs `interval`, refills in real time.
struct Bucket {
    credit: std::time::Duration,
    last: Time,
}

#[test]
fn gcra_matches_a_naive_token_bucket_on_random_traffic() {
    let mut rng = Lcg(0x5eed);
    for (interval, burst) in [(1, 1), (100, 1), (100, 5), (250, 3), (1_000, 10), (7, 13)] {
        let interval = span(interval);
        let rule = gcra(
            |c: &(String, Option<Time>)| c.0.clone(),
            interval,
            burst,
            |c: &(String, Option<Time>), _: &str| c.1,
        );
        let capacity = interval * burst;
        let mut tats: HashMap<String, Time> = HashMap::new();
        let mut buckets: HashMap<String, Bucket> = HashMap::new();
        let mut now: Time = ms(1_767_225_600_000);
        for _ in 0..20_000 {
            now += span(rng.next(3) * rng.next(interval.as_millis() as u64 * 2 + 1));
            let key = format!("k{}", rng.next(3));

            let bucket = buckets.entry(key.clone()).or_insert(Bucket {
                credit: capacity,
                last: now,
            });
            bucket.credit = (bucket.credit + now.since(bucket.last)).min(capacity);
            bucket.last = now;
            let naive_ok = bucket.credit >= interval;
            if naive_ok {
                bucket.credit -= interval;
            }

            let tat = tats.get(&key).copied();
            let result = rule.check(&(key.clone(), tat), now);
            assert_eq!(
                result.is_ready(),
                naive_ok,
                "interval {interval:?} burst {burst} at {now}"
            );
            match result {
                RuleResult::Ready => {
                    tats.insert(key, gcra_spend(tat, now, interval));
                }
                RuleResult::Blocked { wait, .. } => {
                    // At retry_at the naive bucket has exactly one cell of credit.
                    let Wait::At(at) = wait else {
                        panic!("gcra never blocks forever")
                    };
                    assert_eq!(bucket.credit + at.since(now), interval);
                }
            }
        }
    }
}
