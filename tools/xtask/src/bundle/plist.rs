//! The bundle's `Info.plist`: what `LaunchServices` reads to know Quickie is a browser (the
//! `http`/`https` URL types and the web document types it views), what it is called, which
//! version it is, and why it asks for the camera and the microphone.

/// The bundle id of the app users run.
pub const DEFAULT_ID: &str = "dev.quickie.Quickie";

/// The version of an untagged build.
const UNTAGGED: &str = "0.0.0";

/// The oldest macOS the app runs on, on the system `WebKit`.
pub const SYSTEM_MINIMUM: &str = "15.0";

/// What varies between bundles: the id (a test build registers under its own), the version,
/// and the oldest macOS (a bundle of our `WebKit` fork needs the one the fork was built for).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    pub id: String,
    pub version: String,
    pub minimum: String,
}

/// A property list value; enough of the format for an `Info.plist`.
enum Value {
    Text(&'static str),
    Owned(String),
    Flag(bool),
    List(Vec<Value>),
    Dict(Vec<(&'static str, Value)>),
}

/// The version from the latest release tag, `v0.1.0` → `0.1.0`; `0.0.0` without a tag or for a
/// tag that is not a plain `MAJOR.MINOR.PATCH`.
pub fn version_from_tag(tag: Option<&str>) -> String {
    let version = tag.map(str::trim).and_then(|tag| tag.strip_prefix('v'));
    let plain = version.filter(|version| {
        let parts: Vec<&str> = version.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    });
    plain.unwrap_or(UNTAGGED).to_owned()
}

/// True for a bundle id `LaunchServices` accepts: reverse-DNS, letters, digits, `-` and `.`,
/// no empty part.
pub fn valid_id(id: &str) -> bool {
    id.split('.').count() >= 2
        && id.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// The `Info.plist` of `bundle`, as XML.
pub fn info_plist(bundle: &Bundle) -> String {
    let document_type = |name: &'static str, uti: &'static str| {
        Value::Dict(vec![
            ("CFBundleTypeName", Value::Text(name)),
            ("CFBundleTypeRole", Value::Text("Viewer")),
            ("LSHandlerRank", Value::Text("Alternate")),
            ("LSItemContentTypes", Value::List(vec![Value::Text(uti)])),
        ])
    };
    let root = Value::Dict(vec![
        ("CFBundleDevelopmentRegion", Value::Text("en")),
        ("CFBundleDisplayName", Value::Text("Quickie")),
        (
            "CFBundleDocumentTypes",
            Value::List(vec![
                document_type("HTML document", "public.html"),
                document_type("XHTML document", "public.xhtml"),
                document_type("Web archive", "com.apple.webarchive"),
            ]),
        ),
        ("CFBundleExecutable", Value::Text("quickie")),
        ("CFBundleIconFile", Value::Text("Quickie")),
        ("CFBundleIdentifier", Value::Owned(bundle.id.clone())),
        ("CFBundleInfoDictionaryVersion", Value::Text("6.0")),
        ("CFBundleName", Value::Text("Quickie")),
        ("CFBundlePackageType", Value::Text("APPL")),
        (
            "CFBundleShortVersionString",
            Value::Owned(bundle.version.clone()),
        ),
        (
            "CFBundleURLTypes",
            Value::List(vec![Value::Dict(vec![
                ("CFBundleTypeRole", Value::Text("Viewer")),
                ("CFBundleURLName", Value::Text("Web site URL")),
                (
                    "CFBundleURLSchemes",
                    Value::List(vec![Value::Text("http"), Value::Text("https")]),
                ),
            ])]),
        ),
        ("CFBundleVersion", Value::Owned(bundle.version.clone())),
        (
            "LSApplicationCategoryType",
            Value::Text("public.app-category.productivity"),
        ),
        (
            "LSMinimumSystemVersion",
            Value::Owned(bundle.minimum.clone()),
        ),
        (
            "NSCameraUsageDescription",
            Value::Text("A website you visit asks to use the camera."),
        ),
        ("NSHighResolutionCapable", Value::Flag(true)),
        (
            "NSMicrophoneUsageDescription",
            Value::Text("A website you visit asks to use the microphone."),
        ),
        ("NSPrincipalClass", Value::Text("NSApplication")),
    ]);
    document(&root)
}

/// The entitlements of a Developer ID build under the hardened runtime: a site may ask for the
/// camera and the microphone. `WebKit`'s JIT runs in the system's web content process, so the
/// app needs no code-signing exceptions.
pub fn entitlements() -> String {
    document(&Value::Dict(vec![
        ("com.apple.security.device.audio-input", Value::Flag(true)),
        ("com.apple.security.device.camera", Value::Flag(true)),
    ]))
}

/// `root` as a property list document.
fn document(root: &Value) -> String {
    let mut xml = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
        "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n",
    ));
    let mut lines = Vec::new();
    write(root, 0, &mut lines);
    for line in lines {
        xml.push_str(&line);
        xml.push('\n');
    }
    xml.push_str("</plist>\n");
    xml
}

/// `value` as lines of XML, indented by `depth` tabs.
fn write(value: &Value, depth: usize, lines: &mut Vec<String>) {
    let indent = "\t".repeat(depth);
    match value {
        Value::Text(text) => lines.push(format!("{indent}<string>{}</string>", escape(text))),
        Value::Owned(text) => lines.push(format!("{indent}<string>{}</string>", escape(text))),
        Value::Flag(flag) => lines.push(format!("{indent}<{flag}/>")),
        Value::List(items) => {
            lines.push(format!("{indent}<array>"));
            for item in items {
                write(item, depth + 1, lines);
            }
            lines.push(format!("{indent}</array>"));
        }
        Value::Dict(entries) => {
            lines.push(format!("{indent}<dict>"));
            for (key, item) in entries {
                lines.push(format!("{indent}\t<key>{}</key>", escape(key)));
                write(item, depth + 1, lines);
            }
            lines.push(format!("{indent}</dict>"));
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::{Bundle, DEFAULT_ID, entitlements, escape, info_plist, valid_id, version_from_tag};

    #[test]
    fn a_developer_id_build_may_use_the_camera_and_the_microphone_only() {
        let xml = entitlements();
        assert_eq!(after(&xml, "com.apple.security.device.camera"), "<true/>");
        assert_eq!(
            after(&xml, "com.apple.security.device.audio-input"),
            "<true/>"
        );
        assert_eq!(xml.matches("<key>").count(), 2);
    }

    fn plist(id: &str) -> String {
        info_plist(&Bundle {
            id: id.to_owned(),
            version: "1.2.3".to_owned(),
            minimum: "26.5".to_owned(),
        })
    }

    /// The value line right after `<key>{key}</key>`.
    fn after(xml: &str, key: &str) -> String {
        let marker = format!("<key>{key}</key>\n");
        let start = xml.find(&marker).unwrap() + marker.len();
        xml[start..].lines().next().unwrap().trim().to_owned()
    }

    #[test]
    fn the_version_comes_from_a_release_tag() {
        assert_eq!(version_from_tag(Some("v0.1.0")), "0.1.0");
        assert_eq!(version_from_tag(Some("v12.0.31\n")), "12.0.31");
    }

    #[test]
    fn an_untagged_or_odd_tag_is_version_zero() {
        for tag in [
            None,
            Some(""),
            Some("0.1.0"),
            Some("v0.1"),
            Some("v0.1.0-rc1"),
            Some("v0..1"),
            Some("v1.2.3.4"),
        ] {
            assert_eq!(version_from_tag(tag), "0.0.0", "{tag:?}");
        }
    }

    #[test]
    fn bundle_ids_are_reverse_dns() {
        for id in [DEFAULT_ID, "dev.quickie.test", "a.b-c.D9"] {
            assert!(valid_id(id), "{id}");
        }
        for id in [
            "",
            "quickie",
            "dev..quickie",
            "dev.quickie.",
            "dev.quick ie",
            "dev.<q>",
        ] {
            assert!(!valid_id(id), "{id}");
        }
    }

    #[test]
    fn the_plist_names_the_bundle_and_its_version() {
        let xml = plist("dev.quickie.test");

        assert_eq!(
            after(&xml, "CFBundleIdentifier"),
            "<string>dev.quickie.test</string>"
        );
        assert_eq!(
            after(&xml, "CFBundleShortVersionString"),
            "<string>1.2.3</string>"
        );
        assert_eq!(after(&xml, "CFBundleVersion"), "<string>1.2.3</string>");
        assert_eq!(
            after(&xml, "CFBundleExecutable"),
            "<string>quickie</string>"
        );
        assert_eq!(after(&xml, "CFBundleIconFile"), "<string>Quickie</string>");
        assert_eq!(after(&xml, "CFBundlePackageType"), "<string>APPL</string>");
        assert_eq!(
            after(&xml, "LSMinimumSystemVersion"),
            "<string>26.5</string>"
        );
        assert_eq!(after(&xml, "NSHighResolutionCapable"), "<true/>");
        assert_eq!(
            after(&xml, "LSApplicationCategoryType"),
            "<string>public.app-category.productivity</string>"
        );
    }

    #[test]
    fn the_plist_claims_web_links_and_views_web_documents() {
        let xml = plist(DEFAULT_ID);

        let schemes = xml.find("<key>CFBundleURLSchemes</key>").unwrap();
        assert!(xml[schemes..].starts_with(concat!(
            "<key>CFBundleURLSchemes</key>\n",
            "\t\t\t<array>\n",
            "\t\t\t\t<string>http</string>\n",
            "\t\t\t\t<string>https</string>\n",
            "\t\t\t</array>\n",
        )));
        for uti in ["public.html", "public.xhtml", "com.apple.webarchive"] {
            assert!(xml.contains(&format!("<string>{uti}</string>")), "{uti}");
        }
        assert_eq!(xml.matches("<string>Viewer</string>").count(), 4);
    }

    #[test]
    fn the_plist_says_why_it_asks_for_camera_and_microphone() {
        let xml = plist(DEFAULT_ID);

        assert!(after(&xml, "NSCameraUsageDescription").contains("camera"));
        assert!(after(&xml, "NSMicrophoneUsageDescription").contains("microphone"));
    }

    #[test]
    fn the_plist_is_a_closed_xml_document() {
        let xml = plist(DEFAULT_ID);

        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(xml.ends_with("</dict>\n</plist>\n"));
        assert_eq!(
            xml.matches("<dict>").count(),
            xml.matches("</dict>").count()
        );
        assert_eq!(
            xml.matches("<array>").count(),
            xml.matches("</array>").count()
        );
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(escape("a<b>&c"), "a&lt;b&gt;&amp;c");
    }
}
