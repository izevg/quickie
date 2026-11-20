//! The suite's pages, described once: an engine that runs HTML gets them as [`TestPage::html`],
//! the reference engine reads the description itself. Every string here is plain text, with no
//! quote, backslash, or markup, so it goes into HTML and JavaScript as it is.

use std::fmt::Write;

/// Something the suite serves, by its path under [`super::PageSource::base`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// An HTML page.
    Page(TestPage),
    /// A script a page loads, as its source; each of the suite's scripts adds its path and `;`
    /// to `window.ran`.
    Script(&'static str),
    /// A file served as an attachment, so navigating to it downloads it.
    Attachment(&'static str),
    /// An attachment that sends these first bytes and never the rest, so its download keeps
    /// running until it is cancelled.
    Stalled(&'static str),
}

impl Resource {
    /// Its content type.
    #[must_use]
    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Page(_) => "text/html; charset=utf-8",
            Self::Script(_) => "text/javascript",
            Self::Attachment(_) | Self::Stalled(_) => "application/octet-stream",
        }
    }

    /// What is sent of it: all of it, except for a stalled attachment.
    #[must_use]
    pub fn body(&self) -> String {
        match self {
            Self::Page(page) => page.html(),
            Self::Script(text) | Self::Attachment(text) | Self::Stalled(text) => (*text).to_owned(),
        }
    }
}

/// A page: its title and text, the links and buttons on it, and what it does once loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TestPage {
    pub title: &'static str,
    /// Its text, in one paragraph above the controls.
    pub text: &'static str,
    /// Its links and buttons, in document order, each in a paragraph of its own: the link
    /// hints' targets.
    pub controls: &'static [Control],
    /// Paths of the scripts it loads before anything else runs.
    pub scripts: &'static [&'static str],
    /// What it does once loaded, in order, a moment after the load.
    pub on_load: &'static [Act],
}

/// A link or a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// A link to `to`; `blank` asks for a new window (`target=_blank`).
    Link { to: &'static str, blank: bool },
    /// A button that does this when clicked.
    Button(Act),
}

/// What a page does, on load or when a button is clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Sets its title.
    Retitle(&'static str),
    /// `window.open` of a path.
    Open(&'static str),
    /// Clicks its `link`th control, a link, from a script: no modifier key is held, since an
    /// engine takes those only from the user's own input.
    Click(usize),
    /// `window.close()`.
    Close,
    /// `alert`.
    Alert(&'static str),
    /// `confirm`; its title becomes `confirm:true` or `confirm:false`.
    Confirm(&'static str),
    /// `prompt` with a message and a default; its title becomes `prompt:` and the answer, or
    /// `prompt:null` when declined.
    Prompt(&'static str, &'static str),
    /// Asks for the camera; its title becomes `camera:allowed` or `camera:denied`, or
    /// `camera:unavailable` when the page cannot ask at all.
    Camera,
    /// Sets a cookie, `name=value`, for the whole origin.
    SetCookie(&'static str),
    /// Its title becomes `cookies:` and its cookies.
    ShowCookies,
    /// Its title becomes `scripts:` and the path of each script it loaded that ran, each
    /// followed by `;`.
    ShowScripts,
}

impl Act {
    /// The JavaScript that does it, in the page's own world. `window.` is spelled out: in an
    /// inline handler a bare `open` or `close` is the document's.
    fn script(self) -> String {
        match self {
            Self::Retitle(title) => format!("document.title='{title}';"),
            Self::Open(path) => format!("window.open('{path}');"),
            Self::Click(link) => format!("document.getElementById('c{link}').click();"),
            Self::Close => "window.close();".to_owned(),
            Self::Alert(message) => format!("alert('{message}');"),
            Self::Confirm(message) => format!("document.title='confirm:'+confirm('{message}');"),
            Self::Prompt(message, default) => {
                format!("document.title='prompt:'+prompt('{message}','{default}');")
            }
            Self::Camera => "if(!navigator.mediaDevices){document.title='camera:unavailable';}\
                             else{navigator.mediaDevices.getUserMedia({video:true}).then(\
                             ()=>{document.title='camera:allowed'},\
                             ()=>{document.title='camera:denied'});}"
                .to_owned(),
            Self::SetCookie(cookie) => format!("document.cookie='{cookie}; path=/';"),
            Self::ShowCookies => "document.title='cookies:'+document.cookie;".to_owned(),
            Self::ShowScripts => "document.title='scripts:'+(window.ran||'');".to_owned(),
        }
    }
}

impl TestPage {
    /// The page as HTML, for an engine that runs it.
    #[must_use]
    pub fn html(&self) -> String {
        let mut html = format!(
            "<!doctype html><meta charset=\"utf-8\"><title>{}</title>",
            self.title
        );
        for script in self.scripts {
            let _ = write!(html, "<script src=\"{script}\"></script>");
        }
        let _ = write!(html, "<p>{}</p>", self.text);
        for (index, control) in self.controls.iter().enumerate() {
            let _ = match control {
                Control::Link { to, blank } => write!(
                    html,
                    "<p><a id=\"c{index}\" href=\"{to}\"{}>link {index}</a></p>",
                    if *blank { " target=\"_blank\"" } else { "" }
                ),
                Control::Button(act) => write!(
                    html,
                    "<p><button id=\"c{index}\" onclick=\"{}\">button {index}</button></p>",
                    act.script()
                ),
            };
        }
        let on_load: String = self.on_load.iter().map(|act| act.script()).collect();
        let _ = write!(
            html,
            "<script>addEventListener('load',()=>setTimeout(()=>{{{on_load}}},50));</script>"
        );
        html
    }
}

/// A page with nothing on it but its title and text.
const fn plain(title: &'static str, text: &'static str) -> TestPage {
    TestPage {
        title,
        text,
        controls: &[],
        scripts: &[],
        on_load: &[],
    }
}

/// A page that does `on_load` and nothing else.
const fn acting(title: &'static str, on_load: &'static [Act]) -> TestPage {
    TestPage {
        title,
        text: "",
        controls: &[],
        scripts: &[],
        on_load,
    }
}

/// Every resource the suite loads, by path.
pub const SITE: &[(&str, Resource)] = &[
    (
        "plain",
        Resource::Page(plain("Plain", "alpha beta Alpha gamma alpha")),
    ),
    ("other", Resource::Page(plain("Other", "another page"))),
    (
        "late",
        Resource::Page(acting("Early", &[Act::Retitle("Late")])),
    ),
    (
        "opener",
        Resource::Page(TestPage {
            title: "Opener",
            text: "",
            controls: &[
                Control::Link {
                    to: "popup",
                    blank: true,
                },
                Control::Link {
                    to: "other",
                    blank: false,
                },
                Control::Button(Act::Open("popup")),
                Control::Button(Act::Click(0)),
                Control::Button(Act::Retitle("Clicked")),
            ],
            scripts: &[],
            on_load: &[],
        }),
    ),
    (
        "popup",
        Resource::Page(TestPage {
            title: "Popup",
            text: "",
            controls: &[Control::Button(Act::Close)],
            scripts: &[],
            on_load: &[],
        }),
    ),
    (
        "set-cookie",
        Resource::Page(TestPage {
            title: "Jar",
            text: "",
            controls: &[Control::Button(Act::Open("cookies"))],
            scripts: &[],
            on_load: &[Act::SetCookie("jar=a"), Act::ShowCookies],
        }),
    ),
    (
        "cookies",
        Resource::Page(acting("Jar", &[Act::ShowCookies])),
    ),
    (
        "alert",
        Resource::Page(acting(
            "Alert",
            &[Act::Alert("Hello"), Act::Retitle("Alerted")],
        )),
    ),
    (
        "confirm",
        Resource::Page(acting("Confirm", &[Act::Confirm("Sure")])),
    ),
    (
        "prompt",
        Resource::Page(acting("Prompt", &[Act::Prompt("Name", "Ada")])),
    ),
    ("camera", Resource::Page(acting("Camera", &[Act::Camera]))),
    (
        "tracked",
        Resource::Page(TestPage {
            title: "Tracked",
            text: "",
            controls: &[],
            scripts: &["app.js", "tracker.js"],
            on_load: &[Act::ShowScripts],
        }),
    ),
    (
        "app.js",
        Resource::Script("window.ran = (window.ran || '') + 'app.js;';"),
    ),
    (
        "tracker.js",
        Resource::Script("window.ran = (window.ran || '') + 'tracker.js;';"),
    ),
    (
        "report.bin",
        Resource::Attachment("conformance report, whole\n"),
    ),
    (
        "stalled.bin",
        Resource::Stalled("conformance report, the start"),
    ),
];

/// The resource at `path`.
#[must_use]
pub(super) fn resource(path: &str) -> Option<Resource> {
    SITE.iter()
        .find(|(served, _)| *served == path)
        .map(|(_, resource)| *resource)
}
