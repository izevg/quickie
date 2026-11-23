use super::{Response, Transport, download, fetch_latest};
use crate::{Distribution, Version};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use url::Url;

const RELEASE: &[u8] = br#"{"tag_name":"v0.3.0","draft":false,"prerelease":false,"assets":[{"name":"Quickie-0.3.0.dmg","size":5,"browser_download_url":"https://github.com/izevg/quickie/releases/download/v0.3.0/Quickie-0.3.0.dmg","digest":"sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"}]}"#;

struct Fixture(RefCell<VecDeque<(Response, &'static [u8])>>);

impl Transport for Fixture {
    fn get(
        &self,
        _: &Url,
        _: &str,
        body: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<Response, String> {
        let (response, bytes) = self.0.borrow_mut().pop_front().expect("unexpected request");
        if response.status == 200 {
            for part in bytes.chunks(2) {
                body(part).map_err(|error| error.to_string())?;
            }
        }
        Ok(response)
    }
}

fn response(status: u16, location: Option<&str>, body: &'static [u8]) -> (Response, &'static [u8]) {
    (
        Response {
            status,
            location: location.map(str::to_owned),
        },
        body,
    )
}

#[test]
fn downloads_from_the_release_cdn_and_verifies_the_file() {
    let fixture = Fixture(RefCell::new(VecDeque::from([
        response(200, None, RELEASE),
        response(
            302,
            Some("https://release-assets.githubusercontent.com/file?signature=value"),
            b"",
        ),
        response(200, None, b"hello"),
    ])));
    let asset = fetch_latest(&fixture, &Version::new(0, 2, 0), Distribution::MacosArm64)
        .expect("check succeeds")
        .expect("newer release");
    let directory = download(&fixture, &asset).expect("verify");
    assert_eq!(
        std::fs::read(directory.path().join(asset.name())).expect("asset"),
        b"hello"
    );
}

#[test]
fn refuses_untrusted_redirects_before_the_next_request() {
    let fixture = Fixture(RefCell::new(VecDeque::from([
        response(200, None, RELEASE),
        response(
            302,
            Some("https://release-assets.githubusercontent.com.example.org/file"),
            b"",
        ),
    ])));
    let asset = fetch_latest(&fixture, &Version::new(0, 2, 0), Distribution::MacosArm64)
        .expect("check succeeds")
        .expect("newer release");
    assert!(download(&fixture, &asset).is_err());
}

#[test]
fn a_full_length_corrupted_download_cannot_be_staged() {
    let fixture = Fixture(RefCell::new(VecDeque::from([
        response(200, None, RELEASE),
        response(200, None, b"jello"),
    ])));
    let asset = fetch_latest(&fixture, &Version::new(0, 2, 0), Distribution::MacosArm64)
        .expect("check succeeds")
        .expect("newer release");
    assert!(download(&fixture, &asset).is_err());
}
