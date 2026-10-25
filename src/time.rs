//! Time: [`Time`] is a point on the UTC timeline, `std::time::Duration` is a
//! span. They are different types, so a span can never be passed where a
//! point is meant.

use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MICROS_PER_SEC: i64 = 1_000_000;
const MICROS_PER_DAY: i64 = 86_400 * MICROS_PER_SEC;

/// A point in time: microseconds since the Unix epoch, UTC.
///
/// Microseconds match Postgres `timestamptz` and are fine enough for a token
/// bucket; `i64` covers roughly ±292 000 years. Arithmetic with a
/// [`Duration`] saturates at [`Time::MIN`] and [`Time::MAX`].
///
/// ```
/// use std::time::Duration;
/// use pacer::Time;
///
/// let t: Time = "2026-10-25T01:30:00Z".parse().unwrap();
/// assert_eq!(t + Duration::from_secs(90), "2026-10-25T01:31:30Z".parse().unwrap());
/// assert_eq!(t.to_string(), "2026-10-25T01:30:00Z");
/// assert_eq!(Time::from_millis(1_500).as_millis(), 1_500);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Time(i64);

impl Time {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Time = Time(0);
    /// The earliest representable time.
    pub const MIN: Time = Time(i64::MIN);
    /// The latest representable time.
    pub const MAX: Time = Time(i64::MAX);

    /// From microseconds since the epoch.
    pub const fn from_micros(micros: i64) -> Self {
        Time(micros)
    }
    /// From milliseconds since the epoch (saturating).
    pub const fn from_millis(millis: i64) -> Self {
        Time(millis.saturating_mul(1_000))
    }
    /// From seconds since the epoch (saturating).
    pub const fn from_secs(secs: i64) -> Self {
        Time(secs.saturating_mul(MICROS_PER_SEC))
    }
    /// Microseconds since the epoch.
    pub const fn as_micros(self) -> i64 {
        self.0
    }
    /// Milliseconds since the epoch, rounded down (towards the past).
    pub const fn as_millis(self) -> i64 {
        self.0.div_euclid(1_000)
    }
    /// Seconds since the epoch, rounded down (towards the past).
    pub const fn as_secs(self) -> i64 {
        self.0.div_euclid(MICROS_PER_SEC)
    }

    /// `self + span`, `None` past [`Time::MAX`].
    pub fn checked_add(self, span: Duration) -> Option<Time> {
        self.0.checked_add(span_micros(span)?).map(Time)
    }
    /// `self - span`, `None` before [`Time::MIN`].
    pub fn checked_sub(self, span: Duration) -> Option<Time> {
        self.0.checked_sub(span_micros(span)?).map(Time)
    }
    /// `self + span`, saturating.
    pub fn saturating_add(self, span: Duration) -> Time {
        self.checked_add(span).unwrap_or(Time::MAX)
    }
    /// `self - span`, saturating.
    pub fn saturating_sub(self, span: Duration) -> Time {
        self.checked_sub(span).unwrap_or(Time::MIN)
    }

    /// The span from `earlier` to `self`; `None` if `earlier` is later.
    pub fn checked_since(self, earlier: Time) -> Option<Duration> {
        let micros = self.0.checked_sub(earlier.0)?;
        u64::try_from(micros).ok().map(Duration::from_micros)
    }
    /// The span from `earlier` to `self`; zero if `earlier` is later.
    pub fn since(self, earlier: Time) -> Duration {
        self.checked_since(earlier).unwrap_or(Duration::ZERO)
    }

    /// Rounded down to a multiple of `unit` on the UTC timeline (e.g. the
    /// start of the UTC hour or day). Panics if `unit` is zero.
    ///
    /// ```
    /// use std::time::Duration;
    /// use pacer::Time;
    ///
    /// let t: Time = "2026-10-25T13:45:10Z".parse().unwrap();
    /// assert_eq!(t.truncate(Duration::from_secs(3600)).to_string(), "2026-10-25T13:00:00Z");
    /// ```
    pub fn truncate(self, unit: Duration) -> Time {
        let unit = span_micros(unit).unwrap_or(i64::MAX);
        assert!(unit > 0, "truncate unit must be positive");
        Time(self.0 - self.0.rem_euclid(unit))
    }

    /// The current system time. Prefer a [`Clock`](crate::Clock) at the
    /// edge of your program; rules always take time as a parameter.
    pub fn now() -> Time {
        Time::from(SystemTime::now())
    }
}

/// A span in microseconds, `None` beyond `i64::MAX`.
pub(crate) fn span_micros(span: Duration) -> Option<i64> {
    i64::try_from(span.as_micros()).ok()
}

impl Add<Duration> for Time {
    type Output = Time;
    /// Saturating.
    fn add(self, span: Duration) -> Time {
        self.saturating_add(span)
    }
}
impl AddAssign<Duration> for Time {
    fn add_assign(&mut self, span: Duration) {
        *self = *self + span;
    }
}
impl Sub<Duration> for Time {
    type Output = Time;
    /// Saturating.
    fn sub(self, span: Duration) -> Time {
        self.saturating_sub(span)
    }
}
impl SubAssign<Duration> for Time {
    fn sub_assign(&mut self, span: Duration) {
        *self = *self - span;
    }
}

impl From<SystemTime> for Time {
    fn from(t: SystemTime) -> Time {
        match t.duration_since(UNIX_EPOCH) {
            Ok(after) => Time::UNIX_EPOCH.saturating_add(after),
            Err(before) => Time::UNIX_EPOCH.saturating_sub(before.duration()),
        }
    }
}
impl From<Time> for SystemTime {
    fn from(t: Time) -> SystemTime {
        match u64::try_from(t.0) {
            Ok(micros) => UNIX_EPOCH + Duration::from_micros(micros),
            Err(_) => UNIX_EPOCH - Duration::from_micros(t.0.unsigned_abs()),
        }
    }
}

#[cfg(feature = "tz")]
impl From<jiff::Timestamp> for Time {
    fn from(t: jiff::Timestamp) -> Time {
        Time(t.as_microsecond())
    }
}
#[cfg(feature = "tz")]
impl From<Time> for jiff::Timestamp {
    /// Clamped to jiff's range (years −9999 to 9999).
    fn from(t: Time) -> jiff::Timestamp {
        let clamped = t.0.clamp(
            jiff::Timestamp::MIN.as_microsecond(),
            jiff::Timestamp::MAX.as_microsecond(),
        );
        jiff::Timestamp::from_microsecond(clamped).expect("clamped into range")
    }
}

#[cfg(feature = "chrono")]
impl<Tz: chrono::TimeZone> From<chrono::DateTime<Tz>> for Time {
    fn from(t: chrono::DateTime<Tz>) -> Time {
        Time(t.timestamp_micros())
    }
}
#[cfg(feature = "chrono")]
impl From<Time> for chrono::DateTime<chrono::Utc> {
    /// Clamped to chrono's range.
    fn from(t: Time) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_micros(t.0).unwrap_or(if t.0 < 0 {
            chrono::DateTime::<chrono::Utc>::MIN_UTC
        } else {
            chrono::DateTime::<chrono::Utc>::MAX_UTC
        })
    }
}

/// RFC 3339 in UTC with as many fraction digits as needed
/// (`2026-01-01T09:00:00Z`, `…:00.250Z`, `…:00.000001Z`).
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let days = self.0.div_euclid(MICROS_PER_DAY);
        let of_day = self.0.rem_euclid(MICROS_PER_DAY);
        let (y, m, d) = civil_from_days(days);
        let secs = of_day / MICROS_PER_SEC;
        let micros = of_day % MICROS_PER_SEC;
        let (h, mi, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        if (0..=9999).contains(&y) {
            write!(f, "{y:04}")?;
        } else {
            write!(
                f,
                "{}{:06}",
                if y < 0 { '-' } else { '+' },
                y.unsigned_abs()
            )?;
        }
        write!(f, "-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}")?;
        if micros % 1_000 != 0 {
            write!(f, ".{micros:06}")?;
        } else if micros != 0 {
            write!(f, ".{:03}", micros / 1_000)?;
        }
        f.write_str("Z")
    }
}

impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Time({self})")
    }
}

/// Why a string is not an RFC 3339 time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTimeError(String);

impl fmt::Display for ParseTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not an RFC 3339 time: {}", self.0)
    }
}
impl std::error::Error for ParseTimeError {}

/// RFC 3339: `YYYY-MM-DDTHH:MM:SS[.f{1,6}](Z|±HH:MM)`.
impl FromStr for Time {
    type Err = ParseTimeError;
    fn from_str(s: &str) -> Result<Time, ParseTimeError> {
        let bad = || ParseTimeError(s.to_owned());
        let b = s.as_bytes();
        let num = |r: std::ops::Range<usize>| -> Result<i64, ParseTimeError> {
            let part = s.get(r).ok_or_else(bad)?;
            if !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()) {
                part.parse().map_err(|_| bad())
            } else {
                Err(bad())
            }
        };
        if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
            return Err(bad());
        }
        if b[13] != b':' || b[16] != b':' {
            return Err(bad());
        }
        let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
        let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
        if !(1..=12).contains(&m)
            || d < 1
            || d > days_in_month(y, m)
            || h > 23
            || mi > 59
            || sec > 59
        {
            return Err(bad());
        }
        let mut i = 19;
        let mut micros = 0;
        if b.get(i) == Some(&b'.') {
            let start = i + 1;
            let mut end = start;
            while b.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            let digits = end - start;
            if digits == 0 || digits > 6 {
                return Err(bad());
            }
            micros = num(start..end)? * 10_i64.pow((6 - digits) as u32);
            i = end;
        }
        let offset = match b.get(i) {
            Some(b'Z' | b'z') if i + 1 == b.len() => 0,
            Some(sign @ (b'+' | b'-')) if i + 6 == b.len() && b[i + 3] == b':' => {
                let (oh, om) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
                if oh > 23 || om > 59 {
                    return Err(bad());
                }
                let o = (oh * 60 + om) * 60;
                if *sign == b'+' { o } else { -o }
            }
            _ => return Err(bad()),
        };
        let secs = days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + sec - offset;
        Ok(Time(secs * MICROS_PER_SEC + micros))
    }
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_parse_round_trip() {
        for s in [
            "1970-01-01T00:00:00Z",
            "2026-10-25T01:30:00Z",
            "2026-10-25T01:30:00.250Z",
            "2026-10-25T01:30:00.000001Z",
            "1969-12-31T23:59:59.999999Z",
            "2024-02-29T12:00:00Z",
        ] {
            assert_eq!(s.parse::<Time>().unwrap().to_string(), s);
        }
        assert_eq!(
            "2026-10-25T03:30:00+02:00".parse::<Time>().unwrap(),
            "2026-10-25T01:30:00Z".parse::<Time>().unwrap()
        );
        for bad in [
            "2026-02-30T00:00:00Z",
            "2026-10-25 01:30Z",
            "x",
            "2026-10-25T01:30:00.1234567Z",
            "2026-10-25T01:30:00",
        ] {
            assert!(bad.parse::<Time>().is_err(), "{bad}");
        }
    }

    #[test]
    fn units_and_arithmetic() {
        assert_eq!(Time::from_millis(-1).as_micros(), -1_000);
        assert_eq!(Time::from_micros(-1).as_millis(), -1, "floor");
        assert_eq!(Time::MAX + Duration::from_secs(1), Time::MAX);
        assert_eq!(Time::MIN - Duration::from_secs(1), Time::MIN);
        let a = Time::from_secs(10);
        assert_eq!(a.since(Time::from_secs(4)), Duration::from_secs(6));
        assert_eq!(Time::from_secs(4).since(a), Duration::ZERO);
        assert_eq!(Time::from_secs(4).checked_since(a), None);
        assert_eq!(Time::from(SystemTime::from(a)), a);
        assert_eq!(
            Time::from(SystemTime::from(Time::from_micros(-5))),
            Time::from_micros(-5)
        );
    }
}
