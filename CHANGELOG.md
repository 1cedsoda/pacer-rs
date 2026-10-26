# Changelog

## 0.2.1

- Feature `serde`: `Time` as an RFC 3339 string.

## 0.2.0

Time has types now (breaking).

- `Time`: a point, microseconds since the Unix epoch (UTC). RFC 3339
  `Display` and `FromStr`, saturating arithmetic with `std::time::Duration`,
  `since`, `truncate`, conversions with `SystemTime`, `jiff::Timestamp`
  (feature `tz`) and `chrono::DateTime` (feature `chrono`).
- Spans are `std::time::Duration` everywhere; `MillisFn` became `SpanFn`.
  `backoff(.., base, ..).max(..)`, `.retry_after(..)`, `sliding_quota(.., window, ..)`.
- `Clock` (object safe) with `SystemClock`, `MonotonicClock` and `ManualClock`;
  `Pacer::check_now`. Rules still take `now` as a parameter.
- `sim::simulate`: run a pacer through time without waiting; the result prints
  as a timeline (`Simulation::in_zone` for local times).
- Feature `tz` moved from chrono + chrono-tz to jiff with a bundled tz database;
  `pacer::tz::{zone, local, day, next_local, instant}`.
- `TimeWindow::utc_aligned`, `len`, `contains`. Removed the millisecond helpers
  (`Millis`, `DAY_MS`, `start_of_day`, `end_of_hour`, `add_ms`,
  `create_window`, `to_iso_string`).
- Negative spans are impossible by type; the checks that panicked on them are gone.

## 0.1.0

Port of `@1cedsoda/pacer`, plus `not_after`, `zoned_window`, `gcra` and
`Verdict::next_at`.
