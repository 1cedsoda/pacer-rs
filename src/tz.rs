//! Local calendars (cargo feature `tz`): a [`Time`] read in an IANA time zone
//! with [jiff](https://docs.rs/jiff) and its bundled tz database, so a
//! container without `/usr/share/zoneinfo` computes the same days.
//!
//! A local day is not 24 hours: on the switch to summer time it has 23, on
//! the switch back 25. Use [`day`] instead of adding a day's milliseconds.
//!
//! ```
//! use pacer::tz::{LocalTime, day, next_local, zone};
//! use pacer::Time;
//!
//! let berlin = zone("Europe/Berlin");
//! let t: Time = "2026-10-25T10:00:00Z".parse().unwrap();
//! let d = day(&berlin, t);
//! assert_eq!(d.start.to_string(), "2026-10-24T22:00:00Z"); // 00:00 CEST
//! assert_eq!(d.end.to_string(), "2026-10-25T23:00:00Z"); // 00:00 CET
//! assert_eq!(d.len().as_secs(), 25 * 3600);
//! // The next 07:00 in Berlin.
//! assert_eq!(
//!     next_local(&berlin, t, LocalTime::constant(7, 0, 0, 0)).to_string(),
//!     "2026-10-26T06:00:00Z"
//! );
//! ```

use jiff::civil::DateTime;

pub use jiff::civil::{Date, Time as LocalTime, Weekday};
pub use jiff::tz::TimeZone;

use crate::time::Time;
use crate::types::TimeWindow;

/// The time zone named `name` (IANA, e.g. `"Europe/Berlin"`) from the
/// bundled database. Panics on an unknown name: meant for names fixed in
/// code; use [`TimeZone::get`] for input.
pub fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or_else(|e| panic!("unknown time zone {name}: {e}"))
}

/// The local wall-clock date and time of `t` in `tz`.
pub fn local(tz: &TimeZone, t: Time) -> DateTime {
    tz.to_datetime(jiff::Timestamp::from(t))
}

/// The local calendar day around `t` in `tz`: `[00:00, next 00:00)`, 23 or
/// 25 hours long on a DST switch.
pub fn day(tz: &TimeZone, t: Time) -> TimeWindow {
    let date = local(tz, t).date();
    let start = instant(tz, date.to_datetime(LocalTime::midnight()));
    let end = date.tomorrow().map_or(Time::MAX, |d| {
        instant(tz, d.to_datetime(LocalTime::midnight()))
    });
    TimeWindow::new(start, end)
}

/// The first instant strictly after `t` whose local time in `tz` is `at`
/// (a time in a DST gap resolves to the first instant after the gap).
pub fn next_local(tz: &TimeZone, t: Time, at: LocalTime) -> Time {
    let today = local(tz, t).date();
    let mut date = today.yesterday().unwrap_or(today);
    loop {
        let candidate = instant(tz, date.to_datetime(at));
        if candidate > t {
            return candidate;
        }
        match date.tomorrow() {
            Ok(next) => date = next,
            Err(_) => return Time::MAX,
        }
    }
}

/// The earliest instant whose local wall time in `tz` is `>= naive`: an
/// ambiguous time (fall back) is its earlier occurrence, a time in a gap
/// (spring forward) is the first instant after the gap.
pub fn instant(tz: &TimeZone, naive: DateTime) -> Time {
    let ambiguous = tz.to_ambiguous_timestamp(naive);
    if let Ok(t) = ambiguous.unambiguous() {
        return Time::from(t);
    }
    match ambiguous.offset() {
        jiff::tz::AmbiguousOffset::Fold { before, .. } => {
            // The earlier occurrence uses the offset in force before.
            Time::from(before.to_timestamp(naive).expect("in range"))
        }
        _ => {
            // Gap: binary-search the transition. Invariant:
            // local(lo) < naive <= local(hi).
            let guess = Time::from(TimeZone::UTC.to_timestamp(naive).expect("in range"));
            let day = std::time::Duration::from_secs(86_400);
            let (mut lo, mut hi) = ((guess - day).as_micros(), (guess + day).as_micros());
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if local(tz, Time::from_micros(mid)) >= naive {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            Time::from_micros(hi)
        }
    }
}
