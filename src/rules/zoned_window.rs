use chrono::{
    DateTime, Datelike, Days, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday,
};
use chrono_tz::Tz;

use crate::helpers::DAY_MS;
use crate::types::{Millis, Rule, RuleResult, Wait};

/// Open between a local start and end time of day in an IANA time zone,
/// optionally only on some weekdays. Build with [`zoned_window`].
///
/// Requires cargo feature `tz`.
#[derive(Debug, Clone)]
pub struct ZonedWindow {
    name: String,
    tz: Tz,
    start: NaiveTime,
    end: NaiveTime,
    weekdays: [bool; 7],
    reason: String,
}

/// Open from local `start` (inclusive) to local `end` (exclusive) in `tz`.
///
/// - `end < start` is an overnight window (e.g. 22:00–06:00) that closes on the
///   next local day.
/// - Weekdays (see [`ZonedWindow::on_weekdays`], default: every day) refer to the
///   local day on which a window opens.
/// - DST: a start/end time that falls into a gap (spring forward) resolves to the
///   first instant after the gap; an ambiguous time (fall back) resolves to its
///   earlier occurrence. All computations use the real UTC offset of that day.
/// - No allowed weekday → permanently blocked.
///
/// Panics if `start == end`. Default name `"zoned-window"`, default reason
/// `"outside zoned window"`.
///
/// ```
/// use chrono::{NaiveTime, Weekday::*};
/// use pacer::zoned_window;
///
/// let office = zoned_window(
///     chrono_tz::Europe::Berlin,
///     NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
///     NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
/// )
/// .on_weekdays([Mon, Tue, Wed, Thu, Fri]);
///
/// // Saturday 2026-10-24 12:00 CEST → Monday 2026-10-26 09:00 CET (after the DST switch).
/// assert_eq!(office.next_open(1_792_836_000_000), Some(1_793_001_600_000));
/// ```
pub fn zoned_window(tz: Tz, start: NaiveTime, end: NaiveTime) -> ZonedWindow {
    assert!(start != end, "zoned_window start and end must differ");
    ZonedWindow {
        name: "zoned-window".to_owned(),
        tz,
        start,
        end,
        weekdays: [true; 7],
        reason: "outside zoned window".to_owned(),
    }
}

impl ZonedWindow {
    /// Override the rule name.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Use a fixed block reason.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    /// Only open on these local weekdays (of the opening day).
    pub fn on_weekdays<I: IntoIterator<Item = Weekday>>(mut self, days: I) -> Self {
        self.weekdays = [false; 7];
        for day in days {
            self.weekdays[day.num_days_from_monday() as usize] = true;
        }
        self
    }

    /// Whether the window is open at `now`.
    pub fn is_open(&self, now: Millis) -> bool {
        let today = self.local(now).date();
        [today.pred_opt(), Some(today)]
            .into_iter()
            .flatten()
            .filter(|d| self.allowed(*d))
            .any(|d| self.open_at(d) <= now && now < self.close_at(d))
    }

    /// The next opening instant strictly after `now`, or `None` if no weekday
    /// is allowed.
    pub fn next_open(&self, now: Millis) -> Option<Millis> {
        let today = self.local(now).date();
        let first = today.pred_opt().unwrap_or(today);
        (0..=9)
            .filter_map(|i| first.checked_add_days(Days::new(i)))
            .filter(|d| self.allowed(*d))
            .map(|d| self.open_at(d))
            .find(|&open| open > now)
    }

    fn allowed(&self, date: NaiveDate) -> bool {
        self.weekdays[date.weekday().num_days_from_monday() as usize]
    }

    fn open_at(&self, date: NaiveDate) -> Millis {
        self.to_utc(date.and_time(self.start))
    }

    fn close_at(&self, date: NaiveDate) -> Millis {
        let close_date = if self.end > self.start {
            date
        } else {
            date.succ_opt().expect("date out of range")
        };
        self.to_utc(close_date.and_time(self.end))
    }

    fn local(&self, t: Millis) -> NaiveDateTime {
        DateTime::from_timestamp_millis(t)
            .expect("timestamp out of range")
            .with_timezone(&self.tz)
            .naive_local()
    }

    /// Earliest instant whose local wall time is `>= naive`.
    fn to_utc(&self, naive: NaiveDateTime) -> Millis {
        match self.tz.from_local_datetime(&naive) {
            LocalResult::Single(dt) => dt.timestamp_millis(),
            LocalResult::Ambiguous(a, b) => a.timestamp_millis().min(b.timestamp_millis()),
            LocalResult::None => {
                // Gap: binary-search the transition instant. Invariant:
                // local(lo) < naive <= local(hi).
                let guess = naive.and_utc().timestamp_millis();
                let (mut lo, mut hi) = (guess - DAY_MS, guess + DAY_MS);
                while hi - lo > 1 {
                    let mid = lo + (hi - lo) / 2;
                    if self.local(mid) >= naive {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                hi
            }
        }
    }
}

impl<C: ?Sized> Rule<C> for ZonedWindow {
    fn name(&self) -> &str {
        &self.name
    }

    fn check(&self, _ctx: &C, now: Millis) -> RuleResult {
        if self.is_open(now) {
            return RuleResult::Ready;
        }
        let wait = self.next_open(now).map_or(Wait::Never, Wait::At);
        RuleResult::Blocked {
            wait,
            reason: self.reason.clone(),
        }
    }
}
