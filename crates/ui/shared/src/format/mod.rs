//! Quantities and addresses as people read them on glass: sizes, rates, ages, time left, visit
//! counts, URLs. Measured numbers only, rounded the way the Finder rounds them; nothing here
//! invents a value.

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

/// How often a page was visited: `Visited once`, `Visited 6 times`.
#[must_use]
pub fn visits(count: u64) -> String {
    match count {
        1 => "Visited once".to_owned(),
        n => format!("Visited {n} times"),
    }
}

/// An address as people read it: without `https://`, `http://`, or `www.`, without the lone
/// `/` of a site's root, its percent-escapes decoded (`/wiki/Суверенитет`, not
/// `/wiki/%D0%A1…`). An escape that decodes to bytes that are not UTF-8, or to a control or
/// bidirectional formatting character that could disguise the address, keeps the whole
/// address encoded. Long addresses are cut by the label, in the middle, which keeps the host.
#[must_use]
pub fn display_url(url: &str) -> String {
    let rest = ["https://", "http://"]
        .iter()
        .find_map(|scheme| strip_prefix_ignore_case(url, scheme))
        .unwrap_or(url);
    let rest = strip_prefix_ignore_case(rest, "www.").unwrap_or(rest);
    let rest = match rest.find('/') {
        Some(slash) if rest[slash..] == *"/" => &rest[..slash],
        _ => rest,
    };
    match percent_decode(rest) {
        Some(decoded) if decoded.chars().all(readable) => decoded,
        _ => rest.to_owned(),
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// `text` with every `%XX` escape decoded, or `None` when the bytes are not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escape {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Whether `c` may show in an address: not a control character, not a bidirectional or
/// invisible formatting character.
fn readable(c: char) -> bool {
    !c.is_control()
        && !matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

/// `value` with one decimal, or none when it is `.0`.
fn one_decimal(value: f64) -> String {
    let text = format!("{value:.1}");
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{ago, bytes, display_url, rate, time_left, visits};

    #[test]
    fn an_address_drops_its_scheme_and_www() {
        assert_eq!(
            display_url("https://github.com/izevg/quickie/pulls"),
            "github.com/izevg/quickie/pulls"
        );
        assert_eq!(
            display_url("http://www.rust-lang.org/learn"),
            "rust-lang.org/learn"
        );
        assert_eq!(display_url("HTTPS://WWW.Example.com/a"), "Example.com/a");
        assert_eq!(
            display_url("file:///Users/a/b.txt"),
            "file:///Users/a/b.txt"
        );
    }

    #[test]
    fn a_site_root_drops_its_slash() {
        assert_eq!(
            display_url("https://news.ycombinator.com/"),
            "news.ycombinator.com"
        );
        assert_eq!(display_url("https://lenta.ru/news/"), "lenta.ru/news/");
        assert_eq!(display_url("https://a.example/?q=1"), "a.example/?q=1");
    }

    #[test]
    fn escapes_are_decoded_when_they_read_well() {
        assert_eq!(
            display_url(
                "https://ru.wikipedia.org/wiki/%D0%A1%D1%83%D0%B2%D0%B5%D1%80%D0%B5%D0%BD%D0%B8%D1%82%D0%B5%D1%82"
            ),
            "ru.wikipedia.org/wiki/Суверенитет"
        );
        assert_eq!(display_url("https://a.example/a%20b"), "a.example/a b");
        assert_eq!(display_url("https://a.example/100%"), "a.example/100%");
        assert_eq!(display_url("https://a.example/%zz%4"), "a.example/%zz%4");
    }

    #[test]
    fn escapes_that_could_disguise_the_address_stay() {
        assert_eq!(display_url("https://a.example/%FF"), "a.example/%FF");
        assert_eq!(
            display_url("https://a.example/%E2%80%AEtxt"),
            "a.example/%E2%80%AEtxt"
        );
        assert_eq!(display_url("https://a.example/%0A"), "a.example/%0A");
        assert_eq!(
            display_url("https://a.example/%E2%80%8B"),
            "a.example/%E2%80%8B"
        );
    }

    #[test]
    fn visits_are_counted_in_words() {
        assert_eq!(visits(1), "Visited once");
        assert_eq!(visits(6), "Visited 6 times");
        assert_eq!(visits(0), "Visited 0 times");
    }

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
