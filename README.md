# pacer (Rust)

Small, synchronous pacing engine for one class of problem:

> Can I do this now, and if not, when should I retry?

- a `Pacer<C>` is a flat list of `Rule<C>`
- each rule returns `Ready`, a concrete `Wait::At(time)`, or `Wait::Never`
- the pacer runs all rules and takes the latest blocking time

Everything interesting lives in your rules and your context, not in the pacer
itself. This is the Rust port of the TypeScript library
[`@1cedsoda/pacer`](https://github.com/1cedsoda/pacer), see
[Differences](#differences-from-the-typescript-version).

```toml
[dependencies]
pacer = { git = "https://github.com/traumventures/pacer-rs" }
# local calendars (zoned windows, local days) with a bundled tz database:
# pacer = { git = "https://github.com/traumventures/pacer-rs", features = ["tz"] }
```

Features: `tz` (jiff, local calendars), `chrono` (conversions to and from
chrono's `DateTime`).

## Time

Time has types, so a span can never be passed where a point is meant:

| | Type | Meaning |
|---|---|---|
| a point | `pacer::Time` | microseconds since the Unix epoch, UTC (the resolution of Postgres `timestamptz`) |
| a span | `std::time::Duration` | `Duration::from_secs(60)`, never a bare `60_000` |
| a window | `pacer::TimeWindow` | `[start, end)` |

```rust
use std::time::Duration;
use pacer::{Time, TimeWindow};

let t: Time = "2026-10-25T13:45:00Z".parse().unwrap(); // RFC 3339, also with offsets
assert_eq!((t + Duration::from_secs(90)).to_string(), "2026-10-25T13:46:30Z");
assert_eq!(t.since("2026-10-25T13:00:00Z".parse().unwrap()), Duration::from_secs(45 * 60));
assert_eq!(Time::from_millis(1_500).as_millis(), 1_500); // for millisecond code
let hour = TimeWindow::utc_aligned(t, Duration::from_secs(3600));
assert_eq!(hour.start.to_string(), "2026-10-25T13:00:00Z");
```

`Time + Duration` saturates at `Time::MIN`/`Time::MAX`. `Time` converts to and
from `SystemTime`, `jiff::Timestamp` (`tz`) and `chrono::DateTime` (`chrono`).

### Where "now" comes from

Rules never read a clock. They take `now` as a parameter, so they are pure: the
same rules, context and `now` always give the same verdict, and a test needs no
fake. Only the edge of a program reads a `Clock` and passes `now` down:

| Clock | Use |
|---|---|
| `SystemClock` | the wall clock: deadlines, calendar days, anything stored |
| `MonotonicClock` | anchored once, then monotonic: in-memory rate limiters and timeouts, immune to clock jumps |
| `ManualClock` | tests and simulations: `set`, `advance` |

```rust
use std::sync::Arc;
use std::time::Duration;
use pacer::{Clock, ManualClock, Time};

fn handler(clock: &dyn Clock) -> Time {
    clock.now() // pass this down to `Pacer::check`
}

let clock = Arc::new(ManualClock::at("2026-10-25T00:00:00Z".parse().unwrap()));
clock.advance(Duration::from_secs(3600));
assert_eq!(handler(&*clock).to_string(), "2026-10-25T01:00:00Z");
```

`Clock` is object safe (`&dyn Clock`, `Arc<dyn Clock>`), so one clock can be
the only time source of a whole application. `Clock::now_ms` bridges code that
still counts `i64` milliseconds. `Pacer::check_now(ctx, &clock)` checks at the
clock's now.

### Local calendars (feature `tz`)

A local day is not 24 hours: on the switch to summer time it has 23, on the
switch back 25. `pacer::tz` reads times in an IANA zone with jiff and a bundled
tz database (no `/usr/share/zoneinfo` needed):

```rust
# #[cfg(feature = "tz")] {
use pacer::Time;
use pacer::tz::{LocalTime, day, next_local, zone};

let berlin = zone("Europe/Berlin");
let t: Time = "2026-10-25T10:00:00Z".parse().unwrap();
assert_eq!(day(&berlin, t).len().as_secs(), 25 * 3600);
assert_eq!(
    next_local(&berlin, t, LocalTime::constant(7, 0, 0, 0)).to_string(),
    "2026-10-26T06:00:00Z"
);
# }
```

## Core Primitives

```rust,ignore
pub enum Wait { At(Time), Never }

pub enum RuleResult { Ready, Blocked { wait: Wait, reason: String } }

pub trait Rule<C: ?Sized>: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, ctx: &C, now: Time) -> RuleResult;
}

impl<C: ?Sized> Pacer<C> {
    pub fn new(rules: Vec<Box<dyn Rule<C>>>) -> Self;
    pub fn check(&self, ctx: &C, now: Time) -> Verdict;
}

pub enum Verdict {
    Ready,
    /// `wait`: the latest `At` across all blocks, `Never` if any block is `Never`.
    Blocked { wait: Wait, blocks: Vec<Block> },
}

impl Verdict {
    /// Ready -> Some(now), At(t) -> Some(t), Never -> None.
    pub fn next_at(&self, now: Time) -> Option<Time>;
}
```

## Quick Start

Load everything the rules need into the context first, then check. The rules
only read it.

```rust
use std::time::Duration;
use pacer::{Pacer, Time, TimeWindow, Verdict, Wait, fixed_quota, min_gap};

struct Context {
    email: String,
    /// Loaded from your event store before checking.
    last_sent_at: Option<Time>,
    sent_today: u64,
}

let pacer: Pacer<Context> = Pacer::new(vec![
    Box::new(min_gap(
        |c: &Context| format!("email:{}", c.email),
        Duration::from_secs(5 * 60),
        |c: &Context, _: &str| c.last_sent_at,
    )),
    Box::new(fixed_quota(
        |c: &Context| format!("email:{}", c.email),
        50,
        |_: &Context, now| TimeWindow::utc_aligned(now, Duration::from_secs(86_400)),
        |c: &Context, _: &str, _: TimeWindow| c.sent_today,
    )),
]);

let ctx = Context {
    email: "a@example.com".into(),
    last_sent_at: Some("2026-01-01T09:58:00Z".parse().unwrap()),
    sent_today: 3,
};
let verdict = pacer.check(&ctx, "2026-01-01T10:00:00Z".parse().unwrap());
let Verdict::Blocked { wait: Wait::At(at), .. } = verdict else { panic!() };
assert_eq!(at.to_string(), "2026-01-01T10:03:00Z");
```

## Built-In Rules

Every factory takes closures over the context and returns a rule with the
builder methods `.named(..)`, `.with_reason(..)` and `.with_reason_fn(..)`.
Spans are `Duration`; `cooldown`, `min_gap` and `gcra` also take
`Fn(&C) -> Duration`.

| Rule | Ready while | Blocked until |
|---|---|---|
| `min_gap(scope, gap, last)` | no last event, or `now >= last + gap` | `last + gap` |
| `cooldown(scope, cooldown, last_failure)` | no failure, or `now >= failure + cooldown` | `failure + cooldown` |
| `not_before(time)` | `now >= time` | `time` |
| `not_after(deadline)` | no deadline, or `now <= deadline` | `Never` (`"too late"`) |
| `fixed_window(is_open, next_open)` | `is_open(ctx, now)` | `next_open(ctx, now)` |
| `zoned_window(tz, start, end)` (`tz`) | local time in `[start, end)`, optional `.on_weekdays(..)`, overnight windows, DST-correct | the next opening, `Never` without weekdays |
| `fixed_quota(scope, limit, window, count)` | `count < limit` in `window(ctx, now)` | `window.end` |
| `sliding_quota(scope, limit, window, events)` | fewer than `limit` events in `(now - window, now]` | when the `limit`-th newest event ages out |
| `concurrency_limit(scope, max, inflight)` | `inflight < max` | `now + retry_after` (default 5 s, `.retry_after(..)`, `.retry_after_fn(..)`) |
| `backoff(scope, base, attempts, last)` | no attempt, or `now >= last + delay` | `last + min(base × multiplier^(attempts-1), max)` (`.multiplier`, `.max`) |
| `gcra(scope, interval, burst, tat)` | the token bucket has a cell | when the next cell refills |
| `dedupe(scope, completed_at)` | not completed | `Never` |

### `gcra`: keyed rate limits

A token bucket without a refill task (generic cell rate algorithm). A full
bucket allows `burst` cells at once, then one cell every `interval`. The only
state per scope is its theoretical arrival time (TAT). The rule only checks;
after an allowed cell store `gcra_spend(tat, now, interval)`, atomically with
the check (under the same lock). A scope whose `tat <= now` is a full bucket and
may be dropped.

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use pacer::{Clock, Gcra, ManualClock, Rule, RuleResult, Time, gcra, gcra_spend};

const INTERVAL: Duration = Duration::from_millis(100); // 10 per second

struct Hit { ip: String, tat: Option<Time> }

struct Limiter { rule: Gcra<Hit>, tats: Mutex<HashMap<String, Time>> }

impl Limiter {
    fn new() -> Self {
        let rule = gcra(|h: &Hit| format!("ip:{}", h.ip), INTERVAL, 20, |h: &Hit, _: &str| h.tat);
        Limiter { rule: rule.named("http"), tats: Mutex::new(HashMap::new()) }
    }

    /// `Ok(())` to serve the request, `Err(wait)` to answer 429 with Retry-After.
    fn hit(&self, ip: &str, now: Time) -> Result<(), Duration> {
        let mut tats = self.tats.lock().unwrap();
        let hit = Hit { ip: ip.to_owned(), tat: tats.get(ip).copied() };
        match self.rule.check(&hit, now) {
            RuleResult::Ready => {
                tats.insert(hit.ip, gcra_spend(hit.tat, now, INTERVAL));
                Ok(())
            }
            RuleResult::Blocked { wait, .. } => Err(wait.at().expect("gcra never blocks forever").since(now)),
        }
    }
}

// In production a `MonotonicClock`; here a manual one.
let clock = ManualClock::at(Time::UNIX_EPOCH);
let limiter = Limiter::new();
for _ in 0..20 {
    assert!(limiter.hit("203.0.113.7", clock.now()).is_ok());
}
assert_eq!(limiter.hit("203.0.113.7", clock.now()), Err(INTERVAL));
clock.advance(INTERVAL);
assert!(limiter.hit("203.0.113.7", clock.now()).is_ok());
```

## Combinators

- `group(name, rules)`: prefixes every rule name with `"{name}/"`; behaviour unchanged.
- `all_of(name, rules)`: blocked if any child blocks; the latest `At`, `Never` if any child is `Never`.
- `any_of(name, rules)`: ready if any child is ready; otherwise the earliest `At`,
  `Never` only if every child is `Never`.

## Simulating Time

`sim::simulate` runs a pacer over a period without waiting: when it is ready the
action runs (and may change the context), when it blocks the clock jumps to the
retry time. The result prints as a timeline, so a whole schedule is one
readable assertion, also across a DST switch (`Simulation::in_zone`, feature
`tz`).

```rust
use std::time::Duration;
use pacer::{Pacer, Time, backoff, sim::simulate};

struct Attempts(Vec<Time>);

let pacer = Pacer::new(vec![Box::new(
    backoff(
        |_: &Attempts| "job".into(),
        Duration::from_secs(60),
        |a: &Attempts, _: &str| a.0.len() as u32,
        |a: &Attempts, _: &str| a.0.last().copied(),
    )
    .max(Duration::from_secs(300)),
)]);
let from: Time = "2026-01-01T09:00:00Z".parse().unwrap();
let run = simulate(&pacer, &mut Attempts(vec![]), from, from + Duration::from_secs(900), |a, t| a.0.push(t));
assert_eq!(run.to_string(), "\
2026-01-01T09:00:00Z acted
2026-01-01T09:01:00Z acted
2026-01-01T09:03:00Z acted
2026-01-01T09:07:00Z acted
2026-01-01T09:12:00Z acted
2026-01-01T09:15:00Z horizon
");
```

## Driving a Durable Timer

A check is a pure function of `(ctx, now)`, so it fits inside a deterministic
`decide` step. Persist `Verdict::next_at` as the next wake-up of a durable timer;
when it fires, reload the context and decide again.

```rust
use pacer::{Pacer, Time, Verdict};

enum Decision { RunNow, WakeAt(Time), Drop { reasons: Vec<String> } }

fn decide<C>(pacer: &Pacer<C>, ctx: &C, now: Time) -> Decision {
    let verdict = pacer.check(ctx, now);
    match verdict.next_at(now) {
        Some(_) if verdict.is_ready() => Decision::RunNow,
        Some(at) => Decision::WakeAt(at),
        None => Decision::Drop {
            reasons: verdict.blocks().iter().map(|b| format!("{}: {}", b.rule, b.reason)).collect(),
        },
    }
}
# let pacer: Pacer<()> = Pacer::new(vec![]);
# assert!(matches!(decide(&pacer, &(), Time::UNIX_EPOCH), Decision::RunNow));
```

## Writing Custom Rules

Use `rule_fn` or implement `Rule<C>`:

```rust
use std::time::Duration;
use pacer::{Rule, RuleResult, Time, rule_fn};

struct Context { backlog: usize, completed: bool }

struct BacklogShaper;

impl Rule<Context> for BacklogShaper {
    fn name(&self) -> &str {
        "backlog-shaper"
    }
    fn check(&self, ctx: &Context, now: Time) -> RuleResult {
        if ctx.backlog < 100 {
            return RuleResult::Ready;
        }
        RuleResult::blocked_until(now + Duration::from_secs(120), "backlog is too large right now")
    }
}

let once = rule_fn("once", |c: &Context, _now| {
    if c.completed { RuleResult::blocked_forever("already completed") } else { RuleResult::Ready }
});
# let _ = once;
```

## Errors and Restarts

Rules are infallible by signature. Invalid input is a programming error and
panics (`limit == 0`, `burst == 0`, a zero `base`, `window`, `max` or `gcra`
interval, a non-positive `multiplier`, equal `zoned_window` start and end, a
`fixed_window` whose `next_open` is not in the future, an empty `fixed_quota`
window). The pacer does not catch panics or treat errors as ready: load data and
handle I/O errors before you build the context.

The pacer is stateless: all durable state lives in your stores, and a fresh
pacer gives the same verdict for the same context and `now`.

## Migrating from 0.1

| 0.1 | 0.2 |
|---|---|
| `Millis` (`i64` ms) for points and spans | `Time` for points, `Duration` for spans |
| `MillisFn` | `SpanFn` (`Duration` or `Fn(&C) -> Duration`) |
| `backoff(.., base_ms, ..).max_ms(..)` | `backoff(.., base, ..).max(..)` |
| `.retry_after_ms(..)` | `.retry_after(Duration)` |
| `sliding_quota(.., window_ms, ..)` | `sliding_quota(.., window, ..)` |
| `start_of_hour`, `end_of_day`, `create_window`, `add_ms`, `DAY_MS`, ... | `TimeWindow::utc_aligned`, `TimeWindow::new`, `Time + Duration`, `Duration::from_secs` |
| `to_iso_string(t)` | `t.to_string()` (RFC 3339), `"…".parse::<Time>()` |
| feature `tz` on chrono + chrono-tz | feature `tz` on jiff (`pacer::tz`), feature `chrono` for conversions |
| negative spans panicked at check time | impossible: `Duration` is unsigned |

## Differences from the TypeScript version

| | TypeScript `@1cedsoda/pacer` | Rust `pacer` |
|---|---|---|
| Execution | async, accessors may hit a DB | synchronous and pure; load data into the context first |
| Time | `Date` | `Time` (µs, UTC) and `Duration`; a `Clock` only at the edge |
| Permanent block | `retryAt: null` | `Wait::Never` |
| Options | options objects | positional arguments plus builder methods |
| Errors | rules throw | rules panic on invalid input |
| Counts | `number` | unsigned integers |
| New | – | `not_after`, `zoned_window`, `gcra`, `Verdict::next_at`, `sim::simulate`, `pacer::tz` |

## License

MIT
