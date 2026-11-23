use std::fs::File;
use std::io::Read;
use std::path::Path;

use url::Url;

const MAX_NOTES_BYTES: usize = 64 * 1024;

pub(crate) fn read(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if file
        .take((MAX_NOTES_BYTES + 4) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    text[..text.floor_char_boundary(MAX_NOTES_BYTES)].to_owned()
}

pub(crate) fn markup(source: &str, preview: bool) -> String {
    let source = if preview {
        &source[..source.floor_char_boundary(4096)]
    } else {
        source
    };
    let mut output = String::new();
    let mut fenced = false;
    let mut items = 0;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let heading = trimmed
            .strip_prefix('#')
            .and_then(|text| text.trim_start_matches('#').strip_prefix(' '));
        let item = ["- ", "* ", "+ "]
            .into_iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix));
        if preview && items == 3 && (heading.is_some() || item.is_some()) {
            break;
        }
        if fenced {
            styled(&mut output, "tt", line);
        } else if let Some(heading) = heading {
            styled(&mut output, "b", heading);
        } else if let Some(item) = item {
            items += 1;
            output.push_str("• ");
            inline(
                &mut output,
                if preview { without_commit(item) } else { item },
            );
        } else {
            inline(&mut output, line);
        }
        output.push('\n');
    }
    output.trim_end().to_owned()
}

fn without_commit(item: &str) -> &str {
    let Some((text, commit)) = item.rsplit_once(" ([`") else {
        return item;
    };
    let generated = commit
        .split_once("`](https://github.com/izevg/quickie/commit/")
        .and_then(|(short, full)| Some((short, full.strip_suffix("))")?)))
        .is_some_and(|(short, full)| {
            short.len() == 7
                && full.starts_with(short)
                && full.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
    if generated { text } else { item }
}

fn inline(output: &mut String, mut text: &str) {
    while let Some(index) = text.find(['*', '`', '[', '!']) {
        escape(output, &text[..index]);
        let rest = &text[index..];
        if let Some((after, image, label, address)) = link(rest) {
            if !image {
                append_link(output, label, address);
            }
            text = after;
        } else if let Some((tag, content, after)) = inline_style(rest) {
            styled(output, tag, content);
            text = after;
        } else {
            output.push(char::from(rest.as_bytes()[0]));
            text = &rest[1..];
        }
    }
    escape(output, text);
}

fn inline_style(text: &str) -> Option<(&'static str, &str, &str)> {
    for (marker, tag) in [("**", "b"), ("*", "i"), ("`", "tt")] {
        let Some((content, after)) = text
            .strip_prefix(marker)
            .and_then(|text| text.split_once(marker))
        else {
            continue;
        };
        if !content.is_empty() && content.trim() == content {
            return Some((tag, content, after));
        }
    }
    None
}

fn link(text: &str) -> Option<(&str, bool, &str, &str)> {
    let image = text.starts_with("![");
    let text = if image {
        text.strip_prefix("![")?
    } else {
        text.strip_prefix('[')?
    };
    // Bounding a candidate also bounds work on malformed runs of opening brackets.
    let candidate = &text[..text.floor_char_boundary(2048)];
    let (label, rest) = candidate.split_once("](")?;
    if label.contains('[') {
        return None;
    }
    let (address, remainder) = rest.split_once(')')?;
    let after = &text[candidate.len() - remainder.len()..];
    Some((after, image, label, address))
}

fn append_link(output: &mut String, label: &str, address: &str) {
    let url = Url::parse(address).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    });
    let code = label
        .strip_prefix('`')
        .and_then(|label| label.strip_suffix('`'));
    if let Some(url) = &url {
        output.push_str("<a href=\"");
        escape(output, url.as_str());
        output.push_str("\">");
    }
    if let Some(code) = code {
        styled(output, "tt", code);
    } else {
        escape(output, label);
    }
    if url.is_some() {
        output.push_str("</a>");
    }
}

fn styled(output: &mut String, tag: &str, text: &str) {
    output.push('<');
    output.push_str(tag);
    output.push('>');
    escape(output, text);
    output.push_str("</");
    output.push_str(tag);
    output.push('>');
}

fn escape(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            character if character.is_control() && character != '\t' => output.push('�'),
            character => output.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{markup, read};

    #[test]
    fn reads_the_installed_snapshot_without_splitting_utf8_at_the_limit() {
        let mut notes = tempfile::NamedTempFile::new().expect("temporary note snapshot");
        let prefix = "a".repeat(65_535);
        write!(notes, "{prefix}é").expect("write note snapshot");
        assert_eq!(read(notes.path()), prefix);
    }

    #[test]
    fn shows_release_markdown_as_safe_native_markup() {
        let markup = markup(
            "# Features\n- **key** with `code` and *emphasis*.\n<iframe>\n![remote](https://example.com/image.png)",
            false,
        );
        assert!(markup.contains("<b>Features</b>"));
        assert!(markup.contains("• <b>key</b> with <tt>code</tt> and <i>emphasis</i>."));
        assert!(markup.contains("&lt;iframe&gt;"));
        assert!(!markup.contains("https://example.com/image.png"));
    }

    #[test]
    fn limits_preview_items_and_reserves_generated_commit_links_for_all_changes() {
        let source = concat!(
            "### Features\n",
            "- First ([`1abcdef`](https://github.com/izevg/quickie/commit/",
            "1abcdefabcdefabcdefabcdefabcdefabcdefabcd))\n",
            "- Second\n",
            "- Third\n",
            "- Fourth",
        );
        let preview = markup(source, true);
        assert!(preview.contains("First"));
        assert!(preview.contains("Third"));
        assert!(!preview.contains("Fourth"));
        assert!(!preview.contains("1abcdef"));
        let all = markup(source, false);
        assert!(all.contains("Fourth"));
        assert!(all.contains("href=\"https://github.com/izevg/quickie/commit/1abcdef"));
    }

    #[test]
    fn keeps_only_safe_web_links_clickable() {
        let markup = markup(
            "[safe](https://example.com/path?a=1&b=2) [local](file:///etc/passwd) [credentials](https://secret@example.com/)",
            false,
        );
        assert_eq!(markup.matches("href=").count(), 1);
        assert!(markup.contains("a=1&amp;b=2"));
        assert!(markup.contains("local"));
        assert!(!markup.contains("secret@"));
    }
}
