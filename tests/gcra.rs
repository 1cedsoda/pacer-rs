mod common;

use std::collections::HashMap;

use common::ts;
use pacer::{Millis, Rule, RuleResult, Wait, gcra, gcra_spend};

struct Ctx {
    ip: String,
    tat: Option<Millis>,
}

fn ctx(tat: Option<Millis>) -> Ctx {
    Ctx {
        ip: "10.0.0.1".into(),
        tat,
    }
}

fn rule(interval: Millis, burst: u32) -> pacer::Gcra<Ctx> {
    gcra(
        |c: &Ctx| format!("ip:{}", c.ip),
        interval,
        burst,
        |c: &Ctx, _: &str| c.tat,
    )
}

/// Check, and on ready spend one cell. Returns whether the cell passed.
fn take(rule: &pacer::Gcra<Ctx>, state: &mut Ctx, now: Millis, interval: Millis) -> RuleResult {
    let result = rule.check(state, now);
    if result.is_ready() {
        state.tat = Some(gcra_spend(state.tat, now, interval));
    }
    result
}

#[test]
fn gcra_allows_a_burst_then_blocks_with_the_exact_retry_time() {
    let rule = rule(1_000, 3);
    let mut state = ctx(None);
    let now = ts("2026-01-01T10:00:00.000Z");
    for _ in 0..3 {
        assert_eq!(take(&rule, &mut state, now, 1_000), RuleResult::Ready);
    }
    assert_eq!(state.tat, Some(now + 3_000));
    assert_eq!(
        take(&rule, &mut state, now, 1_000),
        RuleResult::blocked_until(now + 1_000, "rate limited (retry in 1s)")
    );
    // Still blocked one millisecond before, ready exactly at retry_at.
    assert!(!rule.check(&state, now + 999).is_ready());
    assert!(rule.check(&state, now + 1_000).is_ready());
}

#[test]
fn gcra_refills_one_cell_per_interval() {
    let rule = rule(500, 2);
    let mut state = ctx(None);
    let now = ts("2026-01-01T10:00:00.000Z");
    assert!(take(&rule, &mut state, now, 500).is_ready());
    assert!(take(&rule, &mut state, now, 500).is_ready());
    assert!(!take(&rule, &mut state, now, 500).is_ready());
    // One interval later exactly one cell is back.
    assert!(take(&rule, &mut state, now + 500, 500).is_ready());
    assert_eq!(
        rule.check(&state, now + 500),
        RuleResult::Blocked {
            wait: Wait::At(now + 1_000),
            reason: "rate limited (retry in 1s)".into()
        }
    );
}

#[test]
fn gcra_treats_a_never_seen_scope_as_a_full_bucket() {
    let rule = rule(60_000, 1);
    assert_eq!(rule.check(&ctx(None), 0), RuleResult::Ready);
    assert_eq!(rule.name(), "gcra");
}

#[test]
fn gcra_treats_a_past_tat_as_a_full_bucket() {
    let rule = rule(1_000, 5);
    let now = ts("2026-01-01T10:00:00.000Z");
    let mut state = ctx(Some(now - 10_000));
    for _ in 0..5 {
        assert!(take(&rule, &mut state, now, 1_000).is_ready());
    }
    assert!(!take(&rule, &mut state, now, 1_000).is_ready());
}

#[test]
fn gcra_with_burst_one_spaces_cells_by_the_interval() {
    let rule = rule(2_500, 1);
    let mut state = ctx(None);
    let now = ts("2026-01-01T10:00:00.000Z");
    assert!(take(&rule, &mut state, now, 2_500).is_ready());
    assert_eq!(
        take(&rule, &mut state, now + 1_000, 2_500),
        RuleResult::blocked_until(now + 2_500, "rate limited (retry in 2s)")
    );
    assert!(take(&rule, &mut state, now + 2_500, 2_500).is_ready());
}

#[test]
fn gcra_accepts_the_interval_as_a_function_of_context() {
    struct Tenant {
        per_second: i64,
        tat: Option<Millis>,
    }
    let rule = gcra(
        |_: &Tenant| "tenant".into(),
        |c: &Tenant| 1_000 / c.per_second,
        2,
        |c: &Tenant, _: &str| c.tat,
    );
    // 4/s → 250 ms interval, tau = 250: tat 1_000 is ready from 750.
    let ctx = Tenant {
        per_second: 4,
        tat: Some(1_000),
    };
    assert_eq!(
        rule.check(&ctx, 700),
        RuleResult::blocked_until(750, "rate limited (retry in 1s)")
    );
    assert!(rule.check(&ctx, 750).is_ready());
}

#[test]
fn gcra_reason_closure_and_name_override() {
    let rule = rule(1_000, 1)
        .named("http")
        .with_reason_fn(|c: &Ctx, now, retry_at, scope| {
            format!("{} {scope} {}", c.ip, retry_at - now)
        });
    assert_eq!(rule.name(), "http");
    assert_eq!(
        rule.check(&ctx(Some(1_000)), 400),
        RuleResult::blocked_until(1_000, "10.0.0.1 ip:10.0.0.1 600")
    );
    let fixed = self::rule(1_000, 1).with_reason("slow down");
    assert_eq!(
        fixed.check(&ctx(Some(1_000)), 400),
        RuleResult::blocked_until(1_000, "slow down")
    );
}

#[test]
fn gcra_spend_advances_from_the_later_of_tat_and_now() {
    assert_eq!(gcra_spend(None, 10_000, 250), 10_250);
    assert_eq!(gcra_spend(Some(9_000), 10_000, 250), 10_250);
    assert_eq!(gcra_spend(Some(12_000), 10_000, 250), 12_250);
}

#[test]
#[should_panic(expected = "burst must be positive")]
fn gcra_panics_on_zero_burst() {
    let _ = rule(1_000, 0);
}

#[test]
#[should_panic(expected = "emission_interval_ms must be positive")]
fn gcra_panics_on_non_positive_interval() {
    let _ = rule(0, 1).check(&ctx(None), 0);
}

#[test]
#[should_panic(expected = "emission_interval_ms must be positive")]
fn gcra_spend_panics_on_non_positive_interval() {
    let _ = gcra_spend(None, 0, -1);
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

/// Naive token bucket: `credit` in milliseconds of refill, capacity
/// `burst * interval`, one cell costs `interval`, refills 1 per millisecond.
struct Bucket {
    credit: Millis,
    last: Millis,
}

#[test]
fn gcra_matches_a_naive_token_bucket_on_random_traffic() {
    let mut rng = Lcg(0x5eed);
    for (interval, burst) in [(1, 1), (100, 1), (100, 5), (250, 3), (1_000, 10), (7, 13)] {
        let rule = gcra(
            |c: &(String, Option<Millis>)| c.0.clone(),
            interval,
            burst,
            |c: &(String, Option<Millis>), _: &str| c.1,
        );
        let capacity = interval * Millis::from(burst);
        let mut tats: HashMap<String, Millis> = HashMap::new();
        let mut buckets: HashMap<String, Bucket> = HashMap::new();
        let mut now: Millis = 1_767_225_600_000;
        for _ in 0..20_000 {
            now += rng.next(3) as Millis * rng.next(interval as u64 * 2 + 1) as Millis;
            let key = format!("k{}", rng.next(3));

            let bucket = buckets.entry(key.clone()).or_insert(Bucket {
                credit: capacity,
                last: now,
            });
            bucket.credit = (bucket.credit + (now - bucket.last)).min(capacity);
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
                "interval {interval} burst {burst} at {now}"
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
                    assert_eq!(bucket.credit + (at - now), interval);
                }
            }
        }
    }
}
