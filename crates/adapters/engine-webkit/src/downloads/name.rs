//! Where a download goes and whether a response is one. Pure, so it is tested without `WebKit`
//! or a disk.

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use url::Url;

/// The name of a download whose server suggested none that is usable.
const FALLBACK: &str = "download";

/// The longest file name, in bytes, that APFS and HFS+ take (255) with room for ` 9999`.
const MAX_NAME_BYTES: usize = 240;

/// How many numbered names are tried before giving up: `a.zip`, `a 2.zip` … `a 9999.zip`.
const MAX_COPIES: u32 = 9999;

/// Whether a response should be saved rather than shown: `WebKit` cannot show its type, or the
/// server asked for it with `Content-Disposition: attachment`.
#[must_use]
pub fn wants_download(can_show: bool, disposition: Option<&str>) -> bool {
    !can_show
        || disposition.is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
        })
}

/// A file name made safe to create in the Downloads folder: no path separators, no control
/// characters, not hidden, not empty, and short enough to take a number.
#[must_use]
pub fn safe_name(suggested: &str) -> String {
    let cleaned: String = suggested
        .trim()
        .chars()
        .map(|c| match c {
            '/' | ':' | '\\' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end();
    if cleaned.is_empty() {
        return FALLBACK.to_owned();
    }
    shorten(cleaned)
}

/// The name a page saved as a PDF gets: its title, else its host, else `page`, made safe, with
/// `.pdf`.
#[cfg(target_os = "macos")]
pub(crate) fn pdf_name(title: Option<&str>, url: &Url) -> String {
    let base = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .or_else(|| url.host_str())
        .unwrap_or("page");
    safe_name(&format!("{base}.pdf"))
}

/// The first of `name`, `name 2`, `name 3` … (the number before the extension, as the Finder
/// numbers copies) in `directory` that `taken` says is free; `None` when none of them is.
#[must_use]
pub fn unique_path(directory: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let (stem, extension) = split(name);
    (1..=MAX_COPIES)
        .map(|copy| {
            if copy == 1 {
                directory.join(name)
            } else {
                directory.join(format!("{stem} {copy}{extension}"))
            }
        })
        .find(|path| !taken(path))
}

/// The name and its extension with the dot (`.zip`), or no extension. A leading dot does not
/// start one, and neither does a trailing dot.
fn split(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `name` cut to [`MAX_NAME_BYTES`] at a character boundary, keeping its extension.
fn shorten(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_owned();
    }
    let (stem, extension) = match split(name) {
        (stem, extension) if extension.len() < MAX_NAME_BYTES / 2 => (stem, extension),
        // An extension that long is no extension: the whole name is cut.
        _ => (name, ""),
    };
    let mut cut = MAX_NAME_BYTES - extension.len();
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{extension}", &stem[..cut])
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    #[cfg(target_os = "macos")]
    use url::Url;

    #[cfg(target_os = "macos")]
    use super::pdf_name;
    use super::{MAX_NAME_BYTES, safe_name, unique_path, wants_download};

    #[cfg(target_os = "macos")]
    #[test]
    fn a_pdf_is_named_after_the_page_title_then_its_host() {
        let page = Url::parse("https://example.com/a").unwrap();
        let start = Url::parse("quickie://start/").unwrap();

        assert_eq!(pdf_name(Some("Docs: Intro"), &page), "Docs_ Intro.pdf");
        assert_eq!(pdf_name(Some("  "), &page), "example.com.pdf");
        assert_eq!(pdf_name(None, &page), "example.com.pdf");
        assert_eq!(pdf_name(None, &start), "start.pdf");
        let blank = Url::parse("about:blank").unwrap();
        assert_eq!(pdf_name(None, &blank), "page.pdf");
    }

    #[test]
    fn a_response_webkit_cannot_show_is_a_download() {
        assert!(wants_download(false, None));
        assert!(!wants_download(true, None));
        assert!(!wants_download(true, Some("inline")));
        assert!(!wants_download(true, Some("inline; filename=a.pdf")));
    }

    #[test]
    fn an_attachment_is_a_download_however_it_is_written() {
        for value in [
            "attachment",
            "Attachment; filename=\"a.pdf\"",
            "  ATTACHMENT ;filename*=UTF-8''a.pdf",
        ] {
            assert!(wants_download(true, Some(value)), "{value}");
        }
        assert!(!wants_download(true, Some("attachments")));
        assert!(!wants_download(true, Some("")));
    }

    #[test]
    fn a_safe_name_has_no_separators_controls_or_leading_dots() {
        assert_eq!(safe_name("report.pdf"), "report.pdf");
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name("a:b\\c\u{0}d\ne.txt"), "a_b_c_d_e.txt");
        assert_eq!(safe_name("  .hidden "), "hidden");
        assert_eq!(safe_name("Отчёт 2024.pdf"), "Отчёт 2024.pdf");
    }

    #[test]
    fn an_unusable_name_becomes_download() {
        for name in ["", "   ", "...", ". ."] {
            assert_eq!(safe_name(name), "download", "{name:?}");
        }
    }

    #[test]
    fn a_long_name_is_cut_at_a_character_keeping_its_extension() {
        let long = format!("{}.pdf", "ё".repeat(200));

        let safe = safe_name(&long);

        assert!(safe.len() <= MAX_NAME_BYTES, "{}", safe.len());
        assert!(safe.len() > MAX_NAME_BYTES - 2, "{}", safe.len());
        assert!(safe.ends_with("ё.pdf"));
        let exact = "a".repeat(MAX_NAME_BYTES);
        assert_eq!(safe_name(&exact), exact);
    }

    #[test]
    fn a_long_extension_is_dropped_rather_than_kept_whole() {
        let name = format!("a.{}", "x".repeat(MAX_NAME_BYTES));

        let safe = safe_name(&name);

        assert_eq!(safe.len(), MAX_NAME_BYTES);
        assert!(safe.starts_with("a.x"));
    }

    #[test]
    fn a_free_name_is_kept() {
        let path = unique_path(Path::new("/d"), "a.zip", |_| false);

        assert_eq!(path, Some(PathBuf::from("/d/a.zip")));
    }

    #[test]
    fn a_taken_name_gets_the_next_free_number_before_its_extension() {
        let taken = [PathBuf::from("/d/a.zip"), PathBuf::from("/d/a 2.zip")];

        let path = unique_path(Path::new("/d"), "a.zip", |path| {
            taken.iter().any(|t| t == path)
        });

        assert_eq!(path, Some(PathBuf::from("/d/a 3.zip")));
    }

    #[test]
    fn names_without_an_extension_are_numbered_at_the_end() {
        let numbered = |name: &str| {
            unique_path(Path::new("/d"), name, |path| {
                path == Path::new("/d").join(name)
            })
        };

        assert_eq!(numbered("README"), Some(PathBuf::from("/d/README 2")));
        assert_eq!(numbered(".profile"), Some(PathBuf::from("/d/.profile 2")));
        assert_eq!(numbered("dots."), Some(PathBuf::from("/d/dots. 2")));
        assert_eq!(numbered("a.tar.gz"), Some(PathBuf::from("/d/a.tar 2.gz")));
    }

    #[test]
    fn no_free_name_is_none() {
        assert_eq!(unique_path(Path::new("/d"), "a.zip", |_| true), None);
    }

    #[test]
    fn the_last_number_tried_is_9999() {
        let path = unique_path(Path::new("/d"), "a", |path| path != Path::new("/d/a 9999"));

        assert_eq!(path, Some(PathBuf::from("/d/a 9999")));
    }
}
