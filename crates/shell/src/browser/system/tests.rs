use std::path::{Path, PathBuf};

use url::Url;

use super::{DefaultBrowserRequest, launch_urls, openable};

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn args(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|&text| text.to_owned()).collect()
}

#[test]
fn the_command_line_opens_urls_and_existing_files() {
    let folder = std::env::temp_dir().join(format!("quickie-launch-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("page.html"), "<p>hi</p>").unwrap();

    let urls = launch_urls(
        &args(&[
            "https://example.com/",
            "-NSDocumentRevisionsDebugMode",
            "YES",
            "page.html",
            "missing.html",
            "mailto:me@example.com",
            "file:///tmp/a.html",
        ]),
        &folder,
    );

    let page = Url::from_file_path(folder.join("page.html")).unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(
        urls,
        [url("https://example.com/"), page, url("file:///tmp/a.html")]
    );
}

#[test]
fn a_bare_launch_opens_nothing() {
    assert_eq!(launch_urls(&[], Path::new("/")), []);
}

#[test]
fn web_pages_and_local_files_open_in_a_tab() {
    for text in [
        "http://127.0.0.1:8080/",
        "https://example.com/a?b#c",
        "file:///Users/me/page.html",
    ] {
        assert!(openable(&url(text)), "{text}");
    }
    for text in [
        "mailto:me@example.com",
        "javascript:alert(1)",
        "ftp://example.com/",
        "quickie://start/",
    ] {
        assert!(!openable(&url(text)), "{text}");
    }
}

#[test]
fn the_bundle_asks_for_http_then_https() {
    let bundle = Path::new("/Applications/Quickie.app");

    let request = DefaultBrowserRequest::for_bundle(bundle);

    assert_eq!(
        request,
        Some(DefaultBrowserRequest {
            app: PathBuf::from("/Applications/Quickie.app"),
            schemes: vec!["http", "https"],
        })
    );
}

#[test]
fn any_app_bundle_may_ask() {
    let request = DefaultBrowserRequest::for_bundle(Path::new("/tmp/build/Quickie Test.APP"));

    assert!(request.is_some());
}

#[test]
fn a_bare_binary_cannot_be_the_default_browser() {
    for folder in [
        "/Users/me/quickie/target/debug",
        "/Users/me/quickie/target/debug/quickie",
        "/Users/me/Quickie.app.d",
        "/",
    ] {
        assert_eq!(
            DefaultBrowserRequest::for_bundle(Path::new(folder)),
            None,
            "{folder}"
        );
    }
}
