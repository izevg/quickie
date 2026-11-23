mod support;

use std::io::{self, Read};

use quickie_updater::{UpdateAsset, UpdateError, select_update};
use serde_json::json;
use support::{MACOS, installed, release, response};

#[expect(
    clippy::expect_used,
    reason = "the update fixture must select an artifact"
)]
fn update() -> UpdateAsset {
    select_update(&response(&release()), &installed(), MACOS)
        .expect("stable release")
        .expect("matched package")
}

#[test]
fn accepts_the_complete_asset_with_the_matching_digest() {
    update()
        .verify(b"hello".as_slice())
        .expect("verified asset");
}

#[test]
fn accepts_uppercase_hex_digits_in_sha256() {
    let mut fixture = release();
    fixture["assets"][0]["digest"] =
        json!("sha256:2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824");
    let asset = select_update(&response(&fixture), &installed(), MACOS)
        .expect("stable release")
        .expect("matched package");

    asset.verify(b"hello".as_slice()).expect("verified asset");
}

#[test]
fn rejects_short_or_trailing_bytes() {
    let asset = update();
    for bytes in [b"hell".as_slice(), b"helloo"] {
        assert!(matches!(
            asset.verify(bytes),
            Err(UpdateError::AssetSizeMismatch)
        ));
    }
}

#[test]
fn refuses_to_read_past_the_first_extra_byte() {
    let mut source = io::Cursor::new(b"hello and far too much more");

    assert!(matches!(
        update().verify(&mut source),
        Err(UpdateError::AssetSizeMismatch)
    ));
    assert_eq!(source.position(), 6);
}

#[test]
fn rejects_tampering_even_with_the_correct_size() {
    assert!(matches!(
        update().verify(b"jello".as_slice()),
        Err(UpdateError::AssetDigestMismatch)
    ));
}

#[test]
fn rejects_streams_that_error_after_the_declared_size() {
    struct FailedStream;
    impl Read for FailedStream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture connection error"))
        }
    }

    assert!(matches!(
        update().verify(b"hello".as_slice().chain(FailedStream)),
        Err(UpdateError::ReadAsset)
    ));
}
