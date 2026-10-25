# pacer (Rust)

Small, synchronous pacing engine for one class of problem:

> Can I do this now, and if not, when should I retry?

This is the Rust port of the TypeScript library
[`@1cedsoda/pacer`](https://github.com/1cedsoda/pacer). See
[Differences from the TypeScript version](#differences-from-the-typescript-version).

The crate is intentionally minimal:

- a `Pacer<C>` is a flat list of `Rule<C>`
- each rule returns either `Ready`, a concrete `Wait::At(ms)`, or `Wait::Never`
- the pacer runs all rules and takes the latest blocking time

Everything interesting lives in your rules and your context, not in the pacer itself.

```toml
[dependencies]
pacer = { git = "https://github.com/1cedsoda/pacer-rs" }
# local-time windows (chrono + chrono-tz):
# pacer = { git = "https://github.com/1cedsoda/pacer-rs", features = ["tz"] }
```

## Mental Model

Think of the pacer as:

1. run every rule
2. collect every block
3. if nothing blocks, return `Verdict::Ready`
4. otherwise return the latest `Wait::At`, or `Wait::Never` if any rule is permanently blocked

That means you can model very different systems with the same primitive:

- message pacing
- per-IP scraping limits
- business hours
- deadlines
- cooldowns after failures
- rolling quotas
- deduplication / one-shot gates
- reservation-backed slot allocation

All time values are `i64` Unix epoch **milliseconds** (`pacer::Millis`).

## Core Primitives

### `Rule<C>`

One pacing rule for one context type. Rules are synchronous and should be pure
over the context and `now`.

```rust,ignore
pub type Millis = i64;

pub enum Wait {
    /// Retry at this epoch-millisecond timestamp.
    At(Millis),
    /// Permanently blocked, never retry.
    Never,
}

pub enum RuleResult {
    Ready,
    Blocked { wait: Wait, reason: String },
}

pub trait Rule<C: ?Sized>: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, ctx: &C, now: Millis) -> RuleResult;
}
```

### `Pacer<C>`

The engine that aggregates rules.

```rust,ignore
impl<C: ?Sized> Pacer<C> {
    pub fn new(rules: Vec<Box<dyn Rule<C>>>) -> Self;
    pub fn check(&self, ctx: &C, now: Millis) -> Verdict;
}
```

### `Verdict`

The combined answer.

```rust,ignore
pub struct Block {
    pub rule: String,
    pub wait: Wait,
    pub reason: String,
}

pub enum Verdict {
    Ready,
    Blocked {
        /// Latest `At` across all blocks; `Never` if any block is `Never`.
        wait: Wait,
        blocks: Vec<Block>,
    },
}

impl Verdict {
    /// Ready -> Some(now), At(t) -> Some(t), Never -> None.
    pub fn next_at(&self, now: Millis) -> Option<Millis>;
}
```

### `Pacer::new`

Build a pacer from a flat list of rules.

```rust
use pacer::{Pacer, RuleResult, rule_fn};

let pacer: Pacer<()> = Pacer::new(vec![Box::new(rule_fn("always", |_: &(), _| RuleResult::Ready))]);
let verdict = pacer.check(&(), 1_767_225_600_000);
assert!(verdict.is_ready());
```

## Quick Start

Load everything the rules need into the context first, then check. The rules
only read it.

```rust
use pacer::{Pacer, Verdict, Wait, end_of_day, fixed_quota, min_gap, start_of_day, to_iso_string, TimeWindow};

struct Context {
    email: String,
    /// Loaded from your event store before checking.
    last_sent_at: Option<i64>,
    sent_today: u64,
}

let pacer: Pacer<Context> = Pacer::new(vec![
    Box::new(
        min_gap(
            |c: &Context| format!("email:{}", c.email),
            8 * 60_000,
            |c: &Context, _scope: &str| c.last_sent_at,
        )
        .named("email-min-gap"),
    ),
    Box::new(
        fixed_quota(
            |c: &Context| format!("email:{}", c.email),
            30,
            |_: &Context, now| TimeWindow::new(start_of_day(now), end_of_day(now)),
            |c: &Context, _scope: &str, _window| c.sent_today,
        )
        .named("email-daily-cap"),
    ),
]);

let now = 1_767_261_600_000; // 2026-01-01T10:00:00Z
let ctx = Context { email: "alice@example.com".into(), last_sent_at: Some(now - 60_000), sent_today: 3 };

match pacer.check(&ctx, now) {
    Verdict::Ready => println!("send now"),
    Verdict::Blocked { wait: Wait::Never, blocks } => println!("permanently blocked: {blocks:?}"),
    Verdict::Blocked { wait: Wait::At(t), blocks } => {
        println!("retry at {} (blocked by {blocks:?})", to_iso_string(t));
    }
}
```

## Built-In Exports

Core:

- `Pacer`, `Rule`, `RuleResult`, `Wait`, `Verdict`, `Block`, `Millis`, `TimeWindow`
- `rule_fn` (closure-backed custom rule)
- `group`
- `any_of`
- `all_of`

Rule factories:

- `min_gap`
- `fixed_window`
- `not_before`
- `not_after` (new)
- `fixed_quota`
- `sliding_quota`
- `cooldown`
- `concurrency_limit`
- `backoff`
- `dedupe`
- `zoned_window` (new, cargo feature `tz`)

Helpers:

- `start_of_hour` / `end_of_hour`: UTC hour boundaries
- `start_of_day` / `end_of_day`: UTC day boundaries
- `add_ms`
- `create_window` / `TimeWindow::new`
- `to_iso_string` (like JS `Date#toISOString`)
- constants `SECOND_MS`, `MINUTE_MS`, `HOUR_MS`, `DAY_MS`

## Built-In Rule Factories

The built-in factories take closure accessors over your context instead of
owning a store. That keeps the crate storage-agnostic and every check pure.

Every factory returns a concrete rule type with these builder methods:

- `.named(name)`: override the default rule name
- `.with_reason(text)`: fixed block reason
- `.with_reason_fn(|ctx, ..| String)`: computed block reason (arguments per rule, see the rustdoc)

Accessor closures that take a scope receive it as `&str`, computed by your
`scope(ctx)` closure.

### `min_gap`

Use when one scope needs a minimum delay between actions.

```rust,ignore
min_gap(scope: Fn(&C) -> String, gap: Millis | Fn(&C) -> Millis, get_last_event_time: Fn(&C, &str) -> Option<Millis>)
```

No last event → ready. Otherwise blocked until `last + gap`.

Use cases:

- minimum delay between messages from one email
- minimum delay between requests from one IP
- minimum delay between retries for one account

### `fixed_window`

Use when actions are only allowed during a known time window.

```rust,ignore
fixed_window(is_open: Fn(&C, Millis) -> bool, next_open: Fn(&C, Millis) -> Millis)
```

`next_open` must be strictly after `now` when the window is closed (panics otherwise).

Use cases:

- business hours only (see also `zoned_window`)
- send only in evenings
- API usage only during a maintenance-safe window

### `zoned_window` (feature `tz`)

Use when the window is defined in local time of an IANA time zone.

```rust,ignore
zoned_window(tz: chrono_tz::Tz, start: NaiveTime, end: NaiveTime).on_weekdays([Mon, Tue, Wed, Thu, Fri])
```

- open from `start` (inclusive) to `end` (exclusive) local time
- `end < start` is an overnight window (for example 22:00–06:00)
- weekdays refer to the local day on which the window opens; default is every day
- DST-correct: each day uses its real UTC offset. A start time inside a
  spring-forward gap opens at the transition; an ambiguous fall-back time uses its
  earlier occurrence
- no allowed weekday → `Wait::Never`
- `ZonedWindow::is_open(now)` and `ZonedWindow::next_open(now)` are public too

```rust,ignore
use chrono::{NaiveTime, Weekday::*};
use pacer::zoned_window;

let office = zoned_window(
    chrono_tz::Europe::Berlin,
    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
    NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
)
.on_weekdays([Mon, Tue, Wed, Thu, Fri]);

// Saturday 2026-10-24 12:00 CEST → Monday 2026-10-26 09:00 CET (after the DST switch).
assert_eq!(office.next_open(1_792_836_000_000), Some(1_793_001_600_000));
```

Use cases:

- call or message people only during their local daytime
- weekday-only business hours
- nightly batch windows

### `not_before`

Use when an action has a single scheduled start time.

```rust,ignore
not_before(get_time: Fn(&C) -> Millis)
```

Use cases:

- scheduled workflow step that must not start early
- spawn staggering: each child gets a different `run_at`
- embargo window: process after a fixed delay from creation

### `not_after`

Use when an action must not happen after a deadline.

```rust,ignore
not_after(get_deadline: Fn(&C) -> Option<Millis>)
```

- no deadline (`None`) → ready
- `now <= deadline` → ready
- `now > deadline` → `Wait::Never`, reason `"too late"`

Use cases:

- an offer or invitation that expires
- a reminder that is pointless after the appointment
- combined with `not_before`: an action allowed only inside `[start, deadline]`

### `fixed_quota`

Use when usage resets on a fixed boundary.

```rust,ignore
fixed_quota(scope, limit: u64, window: Fn(&C, Millis) -> TimeWindow, get_count: Fn(&C, &str, TimeWindow) -> u64)
```

`count >= limit` → blocked until `window.end`. Panics if `limit == 0` or the
window is empty.

Use cases:

- max 30 messages per day
- max 100 requests per hour on the hour
- max 5 actions per calendar minute

### `sliding_quota`

Use when the window rolls continuously.

```rust,ignore
sliding_quota(scope, limit: usize, window_ms: Millis, get_event_times: Fn(&C, &str, Millis /* since */, Millis /* until */) -> Vec<Millis>)
```

Events outside `(now - window_ms, now]` are ignored. When the count reaches the
limit, the rule blocks until the `limit`-th newest event ages out.

Use cases:

- max 20 actions in any rolling 10 minutes
- max 50 requests in any rolling hour
- max 3 retries in any rolling 15 minutes

`fixed_quota` and `sliding_quota` are different:

- fixed quota resets at known boundaries
- sliding quota depends on when old events age out

### `cooldown`

Use when a recent failure should pause a scope for some time.

```rust,ignore
cooldown(scope, cooldown: Millis | Fn(&C) -> Millis, get_last_failure_time: Fn(&C, &str) -> Option<Millis>)
```

Use cases:

- cooldown after login failure
- cooldown after anti-bot detection
- cooldown after a send error
- exponential back-off: `|c: &Ctx| 1_000 * 2_i64.pow(c.attempts)`

### `concurrency_limit`

Use when a scope can only have a limited number of inflight actions.

```rust,ignore
concurrency_limit(scope, max: u64, get_inflight_count: Fn(&C, &str) -> u64)
    .retry_after_ms(2_500)                                  // default 5_000
    .retry_after_fn(|ctx, now, scope, active| 1_000 * active as i64)
```

Use cases:

- only one message send per account at a time
- max 3 concurrent workers per tenant
- max 2 inflight page fetches per IP

### `backoff`

Use when a series of attempts should wait progressively longer between retries.

```rust,ignore
backoff(scope, base_ms: Millis, get_attempts: Fn(&C, &str) -> u32, get_last_attempt_time: Fn(&C, &str) -> Option<Millis>)
    .multiplier(2.0) // default 2.0; 1.0 = flat
    .max_ms(300_000) // default: no cap
```

Zero attempts or no last attempt → ready. The delay formula is
`min(base_ms × multiplier^(attempts − 1), max_ms)`.

| attempts | multiplier=2, base_ms=10s | multiplier=1, base_ms=10s |
|----------|---------------------------|---------------------------|
| 1        | 10s                       | 10s                       |
| 2        | 20s                       | 10s                       |
| 3        | 40s                       | 10s                       |
| 4        | 80s                       | 10s                       |

Use cases:

- retry a failing HTTP request with exponential back-off
- re-queue a workflow step after each failed attempt
- flatten retries to a fixed interval when exponential growth is undesirable

### `dedupe`

Use when an item should only be processed once, ever.

```rust,ignore
dedupe(scope, get_completed_at: Fn(&C, &str) -> Option<Millis>)
```

When `get_completed_at` returns a time the rule returns `Wait::Never`. The pacer
(and any combinator wrapping it) propagates `Never` upward.

Use cases:

- skip a lead that has already been contacted
- gate an idempotency check before expensive work
- prevent a one-shot workflow from running twice

## Combinators and Helpers

### `group(name, rules)`

Pure naming helper. It prefixes rule names:

- `email/min-gap`
- `email/fixed-quota`
- `ip/concurrency-limit`

This is only for organization and debugging. It returns a `Vec<Box<dyn Rule<C>>>`
you can pass to `Pacer::new` or extend with more rules.

### `any_of(name, rules)`

Returns ready if any child rule is ready. Every branch is evaluated.

Wait semantics when all branches block:

- the **earliest** `Wait::At` across branches: retry as soon as any branch might unblock
- `Wait::Never` only when **every** branch is permanently blocked

Reason: `"all branches blocked"`. No branches → ready.

Use when you want branch-style logic:

- any proxy may be available
- any account in a pool may be available
- any slot source may unblock the action

### `all_of(name, rules)`

Returns blocked if any child rule blocks.

Wait semantics when rules block:

- the **latest** `Wait::At`: all rules must be satisfied before retrying
- `Wait::Never` if **any** rule is permanently blocked

Reason: `"<n> rule(s) blocked"`. This is mostly useful for nesting inside
`any_of`. Top-level pacer behavior is already effectively `all_of`.

```rust
use pacer::{Pacer, Rule, all_of, any_of, cooldown, concurrency_limit};

struct Ctx { proxy_a_inflight: u64, proxy_b_failed_at: Option<i64>, proxy_b_inflight: u64 }

let proxy_a: Box<dyn Rule<Ctx>> = Box::new(concurrency_limit(|_: &Ctx| "proxy:a".into(), 1, |c: &Ctx, _: &str| c.proxy_a_inflight));
let proxy_b: Box<dyn Rule<Ctx>> = Box::new(all_of("proxy-b", vec![
    Box::new(concurrency_limit(|_: &Ctx| "proxy:b".into(), 1, |c: &Ctx, _: &str| c.proxy_b_inflight)),
    Box::new(cooldown(|_: &Ctx| "proxy:b".into(), 60_000, |c: &Ctx, _: &str| c.proxy_b_failed_at)),
]));
let pacer = Pacer::new(vec![Box::new(any_of("any-proxy", vec![proxy_a, proxy_b])) as Box<dyn Rule<Ctx>>]);

let ctx = Ctx { proxy_a_inflight: 1, proxy_b_failed_at: None, proxy_b_inflight: 0 };
assert!(pacer.check(&ctx, 0).is_ready());
```

## `Wait::Never`: Permanent Blocks

Some rules can signal that a scope will never be ready: `Wait::Never`.

Rules that can produce `Never`:

- `dedupe`: when the item has already been completed
- `not_after`: when the deadline has passed
- `zoned_window`: when no weekday is allowed
- any custom rule that returns `Wait::Never`

Propagation rules:

- `Pacer::check`: `Never` takes precedence over any concrete `At` in the final `Verdict`
- `all_of`: `Never` in any child → `Never` in the combined result
- `any_of`: `Never` only when **all** branches are permanently blocked; a mix of
  `Never` and `At` yields the earliest `At` (the timed branch may still become ready)

The type system makes you handle it: `Verdict::next_at` returns `None` for `Never`.

## Driving a Durable Timer

Because a check is a pure function of `(ctx, now)`, it fits inside a
deterministic `decide` step. Persist the result of `Verdict::next_at` as the
next wake-up of a durable timer:

```rust
use pacer::{Millis, Pacer, Verdict};

enum Decision {
    RunNow,
    WakeAt(Millis),
    Drop { reasons: Vec<String> },
}

fn decide<C>(pacer: &Pacer<C>, ctx: &C, now: Millis) -> Decision {
    let verdict = pacer.check(ctx, now);
    match verdict.next_at(now) {
        Some(_) if verdict.is_ready() => Decision::RunNow,
        Some(at) => Decision::WakeAt(at), // schedule the durable timer for `at`
        None => Decision::Drop {
            reasons: verdict.blocks().iter().map(|b| format!("{}: {}", b.rule, b.reason)).collect(),
        },
    }
}
# let pacer: Pacer<()> = Pacer::new(vec![]);
# assert!(matches!(decide(&pacer, &(), 0), Decision::RunNow));
```

When the timer fires, reload the context and call `decide` again: another rule
may block by then, and the loop simply schedules the next wake-up.

## Date Helpers

All date helpers operate in **UTC** on epoch milliseconds. Do not rely on them
for local-time boundaries.

| Helper | Returns |
|---|---|
| `start_of_hour(t)` | Start of the current UTC hour |
| `end_of_hour(t)` | Start of the next UTC hour |
| `start_of_day(t)` | Midnight UTC of the current day |
| `end_of_day(t)` | Midnight UTC of the next day |
| `add_ms(t, ms)` | `t + ms` |
| `create_window(start, end)` | `TimeWindow` |
| `to_iso_string(t)` | `2026-01-01T09:00:00.000Z` |

For time-zone-aware windows (for example "09:00–20:00 Europe/Berlin") use
`zoned_window` (feature `tz`), or a time-zone library inside your own
`fixed_window` callbacks.

## Scenarios

The examples below assume the data the rules need was loaded into the context
before the check.

## Scenario 1: Outbound Messaging Per Email

Goal:

- minimum delay between messages from one email
- daily send cap per email

See [Quick Start](#quick-start).

## Scenario 2: Scraping Per IP With Business Hours

Goal:

- minimum delay per IP
- scraping only allowed between 07:00 and 23:00 UTC

```rust
use pacer::{HOUR_MS, Pacer, fixed_window, min_gap, start_of_day};

struct Context { ip: String, last_request_at: Option<i64> }

let pacer: Pacer<Context> = Pacer::new(vec![
    Box::new(min_gap(|c: &Context| format!("ip:{}", c.ip), 3_000, |c: &Context, _: &str| c.last_request_at).named("ip-min-gap")),
    Box::new(
        fixed_window(
            |_: &Context, now| (7..23).contains(&((now - start_of_day(now)) / HOUR_MS)),
            |_: &Context, now| {
                let today_7 = start_of_day(now) + 7 * HOUR_MS;
                if now < today_7 { today_7 } else { today_7 + 24 * HOUR_MS }
            },
        )
        .named("wakeful-hours"),
    ),
]);
# let _ = pacer;
```

## Scenario 3: Rolling Quota Per IP

Goal:

- max 50 actions in any rolling hour

```rust
use pacer::{Pacer, sliding_quota};

struct Context { ip: String, recent_events: Vec<i64> }

let pacer: Pacer<Context> = Pacer::new(vec![Box::new(sliding_quota(
    |c: &Context| format!("ip:{}", c.ip),
    50,
    60 * 60_000,
    |c: &Context, _scope: &str, _since, _until| c.recent_events.clone(),
))]);
# let _ = pacer;
```

## Scenario 4: Cooldown After Failure

Goal:

- stop attempting for 15 minutes after the most recent failure

```rust
use pacer::{Pacer, cooldown};

struct Context { email: String, last_failure_at: Option<i64> }

let pacer: Pacer<Context> = Pacer::new(vec![Box::new(cooldown(
    |c: &Context| format!("email:{}", c.email),
    15 * 60_000,
    |c: &Context, _: &str| c.last_failure_at,
))]);
# let _ = pacer;
```

## Scenario 5: Concurrency Ceiling

Goal:

- only one inflight send per email

```rust
use pacer::{Pacer, concurrency_limit};

struct Context { email: String, inflight: u64 }

let pacer: Pacer<Context> = Pacer::new(vec![Box::new(
    concurrency_limit(|c: &Context| format!("email:{}", c.email), 1, |c: &Context, _: &str| c.inflight)
        .retry_after_ms(5_000),
)]);
# let _ = pacer;
```

## Scenario 6: Exponential Back-off After Failures

Goal:

- wait progressively longer after each failed attempt (1s, 2s, 4s, 8s…), capped at 5 minutes

```rust
use pacer::{Pacer, backoff};

struct Context { job_id: String, attempts: u32, last_attempt_at: Option<i64> }

let pacer: Pacer<Context> = Pacer::new(vec![Box::new(
    backoff(
        |c: &Context| format!("job:{}", c.job_id),
        1_000,
        |c: &Context, _: &str| c.attempts,
        |c: &Context, _: &str| c.last_attempt_at,
    )
    .multiplier(2.0)
    .max_ms(5 * 60_000),
)]);
# let _ = pacer;
```

## Scenario 7: One-Shot Deduplication Gate

Goal:

- skip any lead that has already been contacted

```rust
use pacer::{Pacer, Verdict, Wait, dedupe, min_gap};

struct Context { lead_id: u64, email: String, contacted_at: Option<i64>, last_sent_at: Option<i64> }

let pacer: Pacer<Context> = Pacer::new(vec![
    Box::new(dedupe(|c: &Context| format!("lead:{}", c.lead_id), |c: &Context, _: &str| c.contacted_at).named("already-contacted")),
    Box::new(min_gap(|c: &Context| format!("email:{}", c.email), 10 * 60_000, |c: &Context, _: &str| c.last_sent_at).named("send-gap")),
]);

let ctx = Context { lead_id: 1, email: "a@example.com".into(), contacted_at: Some(0), last_sent_at: None };
match pacer.check(&ctx, 1_000) {
    Verdict::Blocked { wait: Wait::Never, .. } => { /* already contacted: discard, do not reschedule */ }
    Verdict::Blocked { wait: Wait::At(_t), .. } => { /* schedule a retry at _t */ }
    Verdict::Ready => { /* send */ }
}
```

## Scenario 8: Dynamic Budget Split Across Accounts

Goal:

- total tenant budget is shared across active accounts
- each account gets a dynamic daily share

```rust
use pacer::{Pacer, RuleResult, end_of_day, rule_fn};

struct Context { email: String, tenant_daily_budget: u64, active_account_count: u64, sent_today: u64 }

let dynamic_account_quota = rule_fn("dynamic-account-quota", |c: &Context, now| {
    let per_account_limit = (c.tenant_daily_budget / c.active_account_count.max(1)).max(1);
    if c.sent_today < per_account_limit {
        return RuleResult::Ready;
    }
    RuleResult::blocked_until(
        end_of_day(now),
        format!("account share exhausted ({}/{per_account_limit})", c.sent_today),
    )
});

let pacer: Pacer<Context> = Pacer::new(vec![Box::new(dynamic_account_quota)]);
# let _ = pacer;
```

This is the point of the crate: special business logic becomes a rule, not a
pacer feature.

## Scenario 9: Global Failure Breaker

Goal:

- if global failure rate is too high, slow the whole system down

```rust
use pacer::{Pacer, RuleResult, add_ms, rule_fn};

struct Context { global_failure_rate: f64 }

let pacer: Pacer<Context> = Pacer::new(vec![Box::new(rule_fn("global-failure-breaker", |c: &Context, now| {
    if c.global_failure_rate < 0.2 {
        return RuleResult::Ready;
    }
    RuleResult::blocked_until(
        add_ms(now, 5 * 60_000),
        format!("global failure rate too high ({:.1}%)", c.global_failure_rate * 100.0),
    )
}))]);
# let _ = pacer;
```

## Scenario 10: Reservation-Backed Slot Access

Goal:

- use a reservation store to spread work into reserved future slots

Rules are pure, so the reservation attempt happens **before** the check (or
after a `Ready` verdict), and its outcome is part of the context:

```rust
use pacer::{Pacer, RuleResult, rule_fn};

enum Reservation { Active, Unavailable { next_available: i64 } }

struct Context { email: String, reservation: Reservation }

let pacer: Pacer<Context> = Pacer::new(vec![Box::new(rule_fn("reservation-required", |c: &Context, _now| {
    match c.reservation {
        Reservation::Active => RuleResult::Ready,
        Reservation::Unavailable { next_available } => {
            RuleResult::blocked_until(next_available, "no reservation slot available")
        }
    }
}))]);
# let _ = pacer;
```

The pacer stays stateless. Restart safety lives in your reservation store and
lease semantics.

## Writing Custom Rules

If the built-ins do not fit, use `rule_fn` or implement `Rule<C>`.

```rust
use pacer::{Millis, Rule, RuleResult};

struct Context { tenant_id: String, backlog_size: usize, completed: bool }

struct BacklogShaper;

impl Rule<Context> for BacklogShaper {
    fn name(&self) -> &str {
        "backlog-shaper"
    }

    fn check(&self, ctx: &Context, now: Millis) -> RuleResult {
        if ctx.backlog_size < 100 {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(now + 2 * 60_000, "backlog is too large right now")
    }
}
```

Return `Wait::Never` (`RuleResult::blocked_forever`) to signal a permanent block:

```rust
use pacer::{RuleResult, rule_fn};
# struct Context { completed: bool }

let permanent_gate = rule_fn("permanent-gate", |c: &Context, _now| {
    if !c.completed {
        return RuleResult::Ready;
    }
    RuleResult::blocked_forever("already completed")
});
# let _ = permanent_gate;
```

## Error Behavior

Rules are infallible by signature. Invalid input is a programming error and
panics, just like a throwing rule makes the TypeScript `pacer.check()` throw:

- factory arguments: `limit == 0`, `max == 0`, `base_ms <= 0`, `window_ms <= 0`,
  non-positive or non-finite `multiplier`, `max_ms <= 0`, equal `zoned_window`
  start and end
- at check time: negative `gap`/`cooldown`/retry delay, `fixed_window` returning a
  `next_open` that is not in the future, an empty `fixed_quota` window

The pacer does not:

- catch panics
- treat errors as ready
- silently skip failing rules

Blocked pacing and execution failure are different outcomes. Load data (and
handle I/O errors) before you build the context.

## Restart Safety

The pacer is stateless.

That means:

- no pacing-critical state lives inside the pacer instance
- all durable state lives in your stores
- a fresh pacer with the same rules produces the same verdict for the same
  context and `now`

When possible:

- derive quotas from durable events
- derive cooldowns from durable failures
- use lease semantics for reservations and inflight claims

## Practical Guidance

- Keep your rules small and focused.
- Load storage data into the context; keep accessors trivial field reads.
- Prefer plain rules before adding abstractions.
- Use `group(...)` for readability, not for behavior.
- Use `any_of(...)` only when you actually need branch-style logic.
- Keep retry reasons human-readable. They show up in `Verdict::Blocked { blocks, .. }`.
- Handle `Wait::Never` explicitly; `Verdict::next_at` returning `None` means "do not reschedule".

## Differences from the TypeScript version

| | TypeScript `@1cedsoda/pacer` | Rust `pacer` |
|---|---|---|
| Execution | async: rules return `RuleResult \| Promise<RuleResult>`, accessors may hit a DB | **synchronous and pure**: `fn check(&self, ctx: &C, now: Millis) -> RuleResult`; load data into the context first. This lets a check run inside a pure, deterministic `decide` function |
| Time | `Date` | **`i64` epoch milliseconds** (`Millis`) everywhere, no date types in the core |
| Permanent block | `retryAt: null` | **`Wait::Never`** (with `Wait::At(ms)` for concrete times) |
| Options | options objects with optional `name` / `reason` | positional required arguments plus builder methods `.named`, `.with_reason`, `.with_reason_fn`, `.multiplier`, `.max_ms`, `.retry_after_ms`, `.retry_after_fn` |
| Errors | rules throw, `check()` rejects | rules panic on invalid input (see [Error Behavior](#error-behavior)) |
| Counts | `number` (validated non-negative at runtime) | unsigned integers (`u64`, `u32`, `usize`), so negative counts cannot occur |
| `any_of` | runs branches in parallel | evaluates every branch sequentially; same result semantics |
| New: `not_after` | – | deadline rule: `now > deadline` → `Wait::Never` (`"too late"`) |
| New: `zoned_window` | – (use `Intl` in `fixedWindow`) | local-time window with weekdays and correct DST handling (cargo feature `tz`) |
| New: `Verdict::next_at` | – | `Ready → Some(now)`, `At(t) → Some(t)`, `Never → None` for driving timers |
| `ScopeKey` | branded string type | not ported; scopes are plain `String`/`&str` |

## Package Philosophy

The pacer is intentionally boring.

That is the point.

It should be easy to explain:

- rules decide
- pacer aggregates
- context carries everything else

## License

MIT
