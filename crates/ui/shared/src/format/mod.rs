//! Quantities as people read them on glass: sizes, rates, ages, time left. Measured numbers
//! only, rounded the way the Finder rounds them; nothing here invents a value.

/// Units of a size, decimal as the Finder counts them.
const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];

/// A size: `512 bytes`, `4.8 MB`, `240 MB`, `1.2 GB`.
#[must_use]
pub fn bytes(count: u64) -> String {
    if count < 1000 {
        return if count == 1 {
            "1 byte".to_owned()
        } else {
            format!("{count} bytes")
        };
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a size shown with one decimal; the lost precision is far below it"
    )]
    let mut value = count as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{} {}", one_decimal(value), UNITS[unit])
}

/// A transfer rate: `4.2 MB/s`.
#[must_use]
pub fn rate(bytes_per_second: u64) -> String {
    format!("{}/s", bytes(bytes_per_second))
}

/// How long ago something happened, `elapsed_ms` before now: `just now`, `5 min ago`,
/// `3 h ago`, `2 days ago`.
#[must_use]
pub fn ago(elapsed_ms: u64) -> String {
    let minutes = elapsed_ms / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;
    match (minutes, hours, days) {
        (0, _, _) => "just now".to_owned(),
        (m, 0, _) => format!("{m} min ago"),
        (_, h, 0) => format!("{h} h ago"),
        (_, _, 1) => "1 day ago".to_owned(),
        (_, _, d) => format!("{d} days ago"),
    }
}

/// Time left at a rate: `12 s left`, `3 min left`, `2 h left`.
#[must_use]
pub fn time_left(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds} s left"),
        60..3600 => format!("{} min left", seconds.div_ceil(60)),
        _ => format!("{} h left", seconds.div_ceil(3600)),
    }
}

/// `value` with one decimal, or none when it is `.0`.
fn one_decimal(value: f64) -> String {
    let text = format!("{value:.1}");
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{ago, bytes, rate, time_left};

    #[test]
    fn small_sizes_count_bytes() {
        assert_eq!(bytes(0), "0 bytes");
        assert_eq!(bytes(1), "1 byte");
        assert_eq!(bytes(999), "999 bytes");
    }

    #[test]
    fn sizes_are_decimal_with_one_place() {
        assert_eq!(bytes(1000), "1 KB");
        assert_eq!(bytes(4_800_000), "4.8 MB");
        assert_eq!(bytes(184_230_000), "184.2 MB");
        assert_eq!(bytes(240_000_000), "240 MB");
        assert_eq!(bytes(1_200_000_000), "1.2 GB");
        assert_eq!(bytes(999_999), "1 MB");
        assert_eq!(bytes(999_949), "999.9 KB");
        assert_eq!(bytes(u64::MAX), "18446.7 PB");
    }

    #[test]
    fn a_rate_is_a_size_per_second() {
        assert_eq!(rate(4_200_000), "4.2 MB/s");
    }

    #[test]
    fn ages_round_down_to_their_unit() {
        assert_eq!(ago(59_999), "just now");
        assert_eq!(ago(60_000), "1 min ago");
        assert_eq!(ago(3_599_999), "59 min ago");
        assert_eq!(ago(3_600_000), "1 h ago");
        assert_eq!(ago(86_399_999), "23 h ago");
        assert_eq!(ago(86_400_000), "1 day ago");
        assert_eq!(ago(3 * 86_400_000), "3 days ago");
    }

    #[test]
    fn time_left_rounds_up() {
        assert_eq!(time_left(0), "0 s left");
        assert_eq!(time_left(59), "59 s left");
        assert_eq!(time_left(60), "1 min left");
        assert_eq!(time_left(61), "2 min left");
        assert_eq!(time_left(3599), "60 min left");
        assert_eq!(time_left(3600), "1 h left");
        assert_eq!(time_left(3601), "2 h left");
    }
}
