use crate::types::{Millis, TimeWindow};

/// One second in milliseconds.
pub const SECOND_MS: Millis = 1_000;
/// One minute in milliseconds.
pub const MINUTE_MS: Millis = 60 * SECOND_MS;
/// One hour in milliseconds.
pub const HOUR_MS: Millis = 60 * MINUTE_MS;
/// One (UTC) day in milliseconds.
pub const DAY_MS: Millis = 24 * HOUR_MS;

/// Start of the current UTC hour.
pub fn start_of_hour(t: Millis) -> Millis {
    t - t.rem_euclid(HOUR_MS)
}

/// Start of the next UTC hour.
pub fn end_of_hour(t: Millis) -> Millis {
    start_of_hour(t) + HOUR_MS
}

/// Midnight UTC of the current day.
pub fn start_of_day(t: Millis) -> Millis {
    t - t.rem_euclid(DAY_MS)
}

/// Midnight UTC of the next day.
pub fn end_of_day(t: Millis) -> Millis {
    start_of_day(t) + DAY_MS
}

/// `t + ms`.
pub fn add_ms(t: Millis, ms: Millis) -> Millis {
    t + ms
}

/// Create a [`TimeWindow`].
pub fn create_window(start: Millis, end: Millis) -> TimeWindow {
    TimeWindow::new(start, end)
}

/// Format epoch milliseconds like JavaScript `Date#toISOString`
/// (`2026-01-01T09:00:00.000Z`).
pub fn to_iso_string(t: Millis) -> String {
    let days = t.div_euclid(DAY_MS);
    let ms_of_day = t.rem_euclid(DAY_MS);
    let (y, m, d) = civil_from_days(days);
    let (h, rest) = (ms_of_day / HOUR_MS, ms_of_day % HOUR_MS);
    let (mi, rest) = (rest / MINUTE_MS, rest % MINUTE_MS);
    let (s, ms) = (rest / SECOND_MS, rest % SECOND_MS);
    let year = if (0..=9999).contains(&y) {
        format!("{y:04}")
    } else if y < 0 {
        format!("-{:06}", -y)
    } else {
        format!("+{y:06}")
    };
    format!("{year}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z")
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
