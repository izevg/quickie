use quickie_updater::{Distribution, Version};
use serde_json::{Value, json};

pub const MACOS: Distribution = Distribution::MacosArm64;
pub const DMG: &str = "Quickie-0.2.0.dmg";
pub const SHA256: &str = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

pub fn installed() -> Version {
    Version::new(0, 1, 0)
}

pub fn asset(name: &str) -> Value {
    json!({
        "name": name,
        "browser_download_url": format!("https://github.com/izevg/quickie/releases/download/v0.2.0/{name}"),
        "size": 5,
        "digest": SHA256,
    })
}

pub fn release() -> Value {
    json!({
        "tag_name": "v0.2.0",
        "draft": false,
        "prerelease": false,
        "assets": [asset(DMG)],
        "body": "# Highlights\n\n- A new release.\n",
    })
}

pub fn response(release: &Value) -> Vec<u8> {
    release.to_string().into_bytes()
}
