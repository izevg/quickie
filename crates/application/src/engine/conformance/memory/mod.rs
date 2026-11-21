//! [`MemoryEngine`]: the reference engine. Its pages are the suite's own descriptions
//! ([`SITE`](super::SITE)) under a base URL, and they do what those say, one step a turn of the
//! engine's loop ([`MemoryEngine::turn`]), never inside a call: what a call asks for is queued,
//! and the waker runs at the end of a turn while events wait.

mod bundle;
mod page;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use quickie_domain_browsing::{Focus, Jar, JarId, TabId, Video, WindowId};
use quickie_domain_settings::ZoomLevel;
use quickie_domain_shields::Site;
use url::Url;

pub use bundle::{MemoryBlocker, MemoryHttp, MemoryView};
pub use page::MemoryPage;

use super::site::resource;
use super::{Act, Control, Resource};
use crate::{
    Answer, Ask, AskKind, BlockerRules, Dialog, DownloadEvent, DownloadToken, Downloads,
    EngineError, EngineEvent, EnginePort, EventQueue, FindStep, Followed, Found, Fullscreen,
    LinkTarget, Navigation, PageAsks, PageContent, PageRect, PageVideo, PageZoom, Permission,
    ReplyToken, Ticket,
};

/// An engine whose pages live in memory, for the suite and the use cases' tests.
pub struct MemoryEngine {
    /// Where the suite's resources are: a resource at `path` is at `base.join(path)`.
    base: Url,
    pages: BTreeMap<TabId, MemoryPage>,
    events: EventQueue<MemoryPage>,
    wake: Option<Box<dyn Fn()>>,
    /// The cookies of each jar, as their pages set them.
    cookies: HashMap<Jar, Vec<&'static str>>,
    /// The rules its content blockers installed last.
    rules: Rc<RefCell<BlockerRules>>,
    /// The last picture of each page: where it was when it was taken.
    pictures: HashMap<TabId, Url>,
    folder: Option<PathBuf>,
    /// Downloaded files, whole or as far as they came.
    files: HashMap<PathBuf, Vec<u8>>,
    /// The file of each download still running.
    running: HashMap<DownloadToken, PathBuf>,
    /// The last number given to an ask, a request, or a download.
    serial: u64,
    /// The tab whose video floats in a slot.
    floated: Option<TabId>,
}

impl MemoryEngine {
    /// An engine with no pages that serves the suite's resources under `base`, a URL that ends
    /// in `/`.
    #[must_use]
    pub fn new(base: Url) -> Self {
        Self {
            base,
            pages: BTreeMap::new(),
            events: EventQueue::default(),
            wake: None,
            cookies: HashMap::new(),
            rules: Rc::default(),
            pictures: HashMap::new(),
            folder: None,
            files: HashMap::new(),
            running: HashMap::new(),
            serial: 0,
            floated: None,
        }
    }

    /// One turn of the engine's loop: every page loads what it was sent to or does its next
    /// act, unless it waits for an answer; then the waker runs if events wait.
    pub fn turn(&mut self) {
        let tabs: Vec<TabId> = self.pages.keys().copied().collect();
        for tab in tabs {
            self.step(tab);
        }
        if let Some(wake) = self.wake.as_ref().filter(|_| !self.events.is_empty()) {
            wake();
        }
    }

    /// The user clicks the link or button of a tab's page whose frame, as
    /// [`PageContent::link_targets`] gave it, is `at`, holding `⌘` when `command`: a link opens
    /// in a background tab with `⌘`, in a new tab when it asks for a new window, and in its
    /// page otherwise; a button does what it does.
    pub fn click(&mut self, tab: TabId, at: PageRect, command: bool) {
        let index = self
            .pages
            .get(&tab)
            .and_then(|page| targets(page).iter().position(|target| target.frame == at));
        if let Some(index) = index {
            self.press(tab, index, command);
            if let Some(page) = self.pages.get_mut(&tab) {
                page.activated = true;
            }
        }
    }

    /// What a downloaded `file` holds, `None` when there is no such file.
    #[must_use]
    pub fn downloaded(&self, file: &Path) -> Option<&[u8]> {
        self.files.get(file).map(Vec::as_slice)
    }

    /// The renderer of a tab's page dies: the page stops, and shows nothing until reloaded.
    ///
    /// # Errors
    ///
    /// The tab has no page.
    pub fn crash(&mut self, tab: TabId) -> Result<(), EngineError> {
        let page = self.page(tab)?;
        page.acts.clear();
        page.asked = None;
        self.events.push(EngineEvent::Crashed { tab });
        Ok(())
    }

    /// The user plays a video on a tab's page, `video` as it then is; the page reports it. A
    /// tab without a page plays nothing.
    pub fn play(&mut self, tab: TabId, video: Video) {
        if let Some(page) = self.pages.get_mut(&tab) {
            let audible = match video {
                Video::Playing { audible } => audible,
                Video::Paused => page.video.is_some_and(|(_, audible)| audible),
            };
            page.video = Some((video, audible));
            self.events.push(EngineEvent::Video {
                tab,
                video: Some(video),
            });
        }
    }

    /// The tab whose video floats in a slot.
    #[must_use]
    pub fn floated(&self) -> Option<TabId> {
        self.floated
    }

    fn page(&mut self, tab: TabId) -> Result<&mut MemoryPage, EngineError> {
        self.pages
            .get_mut(&tab)
            .ok_or_else(|| EngineError(format!("no page for {tab}")))
    }

    fn ticket(&mut self) -> Ticket {
        self.serial += 1;
        Ticket::new(self.serial)
    }

    /// The suite's resource path `url` names, when it is under the base.
    fn path<'u>(&self, url: &'u Url) -> Option<&'u str> {
        url.as_str().strip_prefix(self.base.as_str())
    }

    fn step(&mut self, tab: TabId) {
        let Some(page) = self.pages.get_mut(&tab) else {
            return;
        };
        if let Some(url) = page.loading.take() {
            let jar = page.jar;
            let path = url.as_str().strip_prefix(self.base.as_str());
            match path.and_then(resource) {
                Some(Resource::Page(shown)) => {
                    let left = page.show(url.clone(), shown);
                    if left.fullscreen {
                        self.events.push(EngineEvent::Fullscreen {
                            tab,
                            state: Fullscreen::Left,
                        });
                    }
                    if left.video {
                        self.events.push(EngineEvent::Video { tab, video: None });
                    }
                    self.events
                        .committed(tab, url, Some(shown.title.to_owned()));
                }
                Some(Resource::Attachment(_) | Resource::Stalled(_)) => self.start(tab, jar, url),
                _ => {}
            }
        } else if page.asked.is_none()
            && let Some(act) = page.acts.pop_front()
        {
            self.act(tab, act);
        }
    }

    fn act(&mut self, tab: TabId, act: Act) {
        let Some(page) = self.pages.get_mut(&tab) else {
            return;
        };
        let jar = page.jar;
        let user = std::mem::take(&mut page.activated);
        match act {
            Act::Fullscreen { .. } if !user => {
                self.events.titled(tab, "fullscreen:denied".to_owned());
            }
            Act::Fullscreen { leave } => {
                page.fullscreen = true;
                if leave {
                    page.acts.push_front(Act::ExitFullscreen);
                }
                self.events.push(EngineEvent::Fullscreen {
                    tab,
                    state: Fullscreen::InWindow,
                });
                self.events.titled(tab, "fullscreen:filled".to_owned());
            }
            Act::ExitFullscreen => self.exit_fullscreen(tab),
            Act::Retitle(title) => self.events.titled(tab, title.to_owned()),
            Act::Open(path) => self.open(tab, at(&self.base, path), Focus::Foreground),
            Act::Click(link) => self.press(tab, link, false),
            Act::Close => self.events.push(EngineEvent::ClosedByPage { tab }),
            Act::Alert(message) => self.ask(
                tab,
                act,
                AskKind::Dialog(Dialog::Alert {
                    message: message.to_owned(),
                }),
            ),
            Act::Confirm(message) => self.ask(
                tab,
                act,
                AskKind::Dialog(Dialog::Confirm {
                    message: message.to_owned(),
                }),
            ),
            Act::Prompt(message, default) => self.ask(
                tab,
                act,
                AskKind::Dialog(Dialog::Prompt {
                    message: message.to_owned(),
                    default: default.to_owned(),
                }),
            ),
            Act::Camera => self.ask(tab, act, AskKind::Permission(Permission::Camera)),
            Act::SetCookie(cookie) => self.cookies.entry(jar).or_default().push(cookie),
            Act::ShowCookies => {
                let cookies = self.cookies.get(&jar).map(|cookies| cookies.join("; "));
                self.events
                    .titled(tab, format!("cookies:{}", cookies.unwrap_or_default()));
            }
            Act::FocusField => page.field_focused = true,
            Act::AddFrame(path) | Act::AddForeignFrame(path) => {
                // A frame plays its page's video as the page's own: the page has a video.
                let plays = matches!(resource(path), Some(Resource::Page(framed))
                if framed.on_load.iter().any(|act| {
                    matches!(act, Act::PlayVideo | Act::PlayVideoInShadow)
                }));
                if plays {
                    self.play(tab, Video::Playing { audible: false });
                }
                self.events.titled(tab, format!("framed:{path}"));
            }
            Act::PlayVideo | Act::PlayVideoInShadow => {
                self.play(tab, Video::Playing { audible: false });
            }
            Act::ShowScripts => {
                let rules = self.rules.borrow();
                let ran = page
                    .shown
                    .as_ref()
                    .map_or_else(String::new, |(shown, described)| {
                        let unshielded = Site::try_from(shown)
                            .is_ok_and(|site| rules.unshielded.contains(&site));
                        described
                            .scripts
                            .iter()
                            .filter(|script| unshielded || !blocks(&rules, &at(&self.base, script)))
                            .flat_map(|script| [script, ";"])
                            .collect()
                    });
                self.events.titled(tab, format!("scripts:{ran}"));
            }
            Act::ShowWidth => {
                let width = (WIDTH / page.zoom.factor()).round();
                self.events.titled(tab, format!("width:{width}"));
            }
        }
    }

    /// The `index`th link or button of a tab's page is clicked, with `⌘` held when `command`.
    fn press(&mut self, tab: TabId, index: usize, command: bool) {
        let Some(page) = self.pages.get_mut(&tab) else {
            return;
        };
        match controls(page).get(index).copied() {
            Some(Control::Link { to, .. }) if command => {
                self.open(tab, at(&self.base, to), Focus::Background);
            }
            Some(Control::Link { to, blank: true }) => {
                self.open(tab, at(&self.base, to), Focus::Foreground);
            }
            Some(Control::Link { to, blank: false }) => page.go(at(&self.base, to)),
            Some(Control::Button(act)) => page.acts.push_front(act),
            None => {}
        }
    }

    /// The page in tab `tab` asks `kind` with `act` and waits for the answer.
    fn ask(&mut self, tab: TabId, act: Act, kind: AskKind) {
        self.serial += 1;
        let token = ReplyToken::new(self.serial);
        if let Some(page) = self.pages.get_mut(&tab) {
            page.asked = Some((token, act));
        }
        self.events.push(EngineEvent::Asked(Ask {
            token,
            tab,
            origin: self.base.clone(),
            kind,
        }));
    }

    /// A page tab `opener` opened, in its jar, waiting to be adopted.
    fn open(&mut self, opener: TabId, url: Url, focus: Focus) {
        let jar = self.pages.get(&opener).map_or(Jar::Main, |page| page.jar);
        self.events.push(EngineEvent::Opened {
            opener,
            page: MemoryPage::new(jar, url.clone()),
            url,
            focus,
        });
    }

    /// Starts downloading `url` for a tab's page into the downloads folder: whole at once,
    /// or, for a stalled resource, its start until it is cancelled.
    fn start(&mut self, tab: TabId, jar: Jar, url: Url) {
        self.serial += 1;
        let token = DownloadToken::new(self.serial);
        let (Some(folder), Some(path)) = (self.folder.as_ref(), self.path(&url)) else {
            self.events
                .push(EngineEvent::Download(DownloadEvent::Refused {
                    token,
                    reason: "no downloads folder".to_owned(),
                }));
            return;
        };
        let file = folder.join(path);
        let resource = resource(path);
        self.files.insert(
            file.clone(),
            resource.map_or_else(Vec::new, |resource| resource.body().into_bytes()),
        );
        self.events
            .push(EngineEvent::Download(DownloadEvent::Started {
                token,
                tab,
                jar,
                url,
                file: file.clone(),
            }));
        if matches!(resource, Some(Resource::Stalled(_))) {
            self.running.insert(token, file);
        } else {
            self.events
                .push(EngineEvent::Download(DownloadEvent::Finished { token }));
        }
    }
}

/// How wide every page view is, in points.
const WIDTH: f64 = 1000.0;

/// The URL of the suite's resource at `path` under `base`.
fn at(base: &Url, path: &str) -> Url {
    base.join(path).unwrap_or_else(|_| base.clone())
}

/// Where the links and buttons of the page a page shows are: a point each, one per row, top to
/// bottom.
fn targets(page: &MemoryPage) -> Vec<LinkTarget> {
    controls(page)
        .iter()
        .zip(0_u32..)
        .map(|(control, row)| LinkTarget {
            frame: PageRect {
                x: 0.0,
                y: f64::from(row),
                width: 1.0,
                height: 1.0,
            },
            link: matches!(control, Control::Link { .. }),
        })
        .collect()
}

/// The links and buttons on the page a page shows.
fn controls(page: &MemoryPage) -> &'static [Control] {
    page.shown.as_ref().map_or(&[], |(_, shown)| shown.controls)
}

/// True when a rule of an enabled list names `script`: the script's URL contains it.
fn blocks(rules: &BlockerRules, script: &Url) -> bool {
    rules.lists.iter().any(|list| {
        rules_in(&list.body)
            .iter()
            .any(|rule| script.as_str().contains(rule.as_str()))
    })
}

/// The rules of a list as the memory engine reads them: its lines that are not comments.
fn rules_in(body: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(body)
        .lines()
        .filter(|line| !line.starts_with('!'))
        .map(str::to_owned)
        .collect()
}

impl EnginePort for MemoryEngine {
    type Page = MemoryPage;

    fn adopt(&mut self, page: MemoryPage, tab: TabId) -> Result<(), EngineError> {
        if self.pages.contains_key(&tab) {
            return Err(EngineError(format!("{tab} already has a page")));
        }
        self.pages.insert(tab, page);
        Ok(())
    }

    fn create(&mut self, tab: TabId, url: &Url, jar: Jar) -> Result<(), EngineError> {
        self.adopt(MemoryPage::new(jar, url.clone()), tab)
    }

    /// A floating page shown again docks.
    fn show(&mut self, _window: WindowId, tab: TabId) -> Result<(), EngineError> {
        self.page(tab)?;
        self.dock_video(tab);
        Ok(())
    }

    fn navigate(&mut self, tab: TabId, navigation: &Navigation) -> Result<(), EngineError> {
        let page = self.page(tab)?;
        match navigation {
            Navigation::Load(url) => page.go(url.clone()),
            Navigation::Back => page.back(),
            Navigation::Forward => page.forward(),
            Navigation::Reload | Navigation::HardReload => page.loading = page.url().cloned(),
            Navigation::Stop => page.loading = None,
        }
        Ok(())
    }

    fn close(&mut self, tab: TabId) -> Result<(), EngineError> {
        self.pages
            .remove(&tab)
            .ok_or_else(|| EngineError(format!("no page for {tab}")))?;
        self.events.forget(tab);
        self.pictures.remove(&tab);
        self.dock_video(tab);
        Ok(())
    }

    fn destroy_jar(&mut self, jar: JarId) -> Result<(), EngineError> {
        self.cookies.remove(&Jar::Throwaway(jar));
        Ok(())
    }
}

/// Its pages' content is their description: the text they show and their controls, one row
/// each.
impl PageContent for MemoryEngine {
    fn find(&mut self, tab: TabId, query: &str, step: FindStep) -> Result<(), EngineError> {
        let matches = self.page(tab)?.find(query, step);
        self.events.push(EngineEvent::Found(Found {
            tab,
            query: query.to_owned(),
            matches,
        }));
        Ok(())
    }

    fn read_text(&mut self, tab: TabId, max_chars: usize) -> Result<Ticket, EngineError> {
        let text = self
            .page(tab)?
            .shown
            .as_ref()
            .map_or("", |(_, shown)| shown.text)
            .chars()
            .take(max_chars)
            .collect();
        let ticket = self.ticket();
        self.events.push(EngineEvent::Text { tab, ticket, text });
        Ok(ticket)
    }

    fn link_targets(&mut self, tab: TabId) -> Result<Ticket, EngineError> {
        let targets = targets(self.page(tab)?);
        let ticket = self.ticket();
        self.events.push(EngineEvent::Targets {
            tab,
            ticket,
            targets,
        });
        Ok(ticket)
    }

    /// A link opens in the background when `background`, in a new tab when it asks for a new
    /// window, and in its page otherwise; a button is clicked.
    fn follow_link(
        &mut self,
        tab: TabId,
        index: usize,
        background: bool,
    ) -> Result<Ticket, EngineError> {
        let base = self.base.clone();
        let page = self.page(tab)?;
        let followed = match controls(page).get(index).copied() {
            Some(Control::Link { to, .. }) if background => Followed::Background(at(&base, to)),
            Some(Control::Link { to, blank: true }) => Followed::NewTab(at(&base, to)),
            Some(Control::Link { to, blank: false }) => {
                page.go(at(&base, to));
                Followed::Done
            }
            Some(Control::Button(act)) => {
                page.acts.push_front(act);
                Followed::Done
            }
            None => Followed::Done,
        };
        let ticket = self.ticket();
        self.events.push(EngineEvent::Followed {
            tab,
            ticket,
            followed,
        });
        Ok(ticket)
    }
}

/// Downloads land in a folder that exists only in memory; a page saved as a PDF is a file named
/// after its title that starts the way every PDF does.
impl Downloads for MemoryEngine {
    fn set_directory(&mut self, directory: PathBuf) {
        self.folder = Some(directory);
    }

    fn download(&mut self, tab: TabId, url: &Url) -> Result<(), EngineError> {
        let jar = self.page(tab)?.jar;
        self.start(tab, jar, url.clone());
        Ok(())
    }

    fn cancel(&mut self, token: DownloadToken) {
        if let Some(file) = self.running.remove(&token) {
            self.files.remove(&file);
        }
    }

    fn save_pdf(&mut self, tab: TabId) -> Result<(), EngineError> {
        let page = self.page(tab)?;
        let jar = page.jar;
        let (url, shown) = page
            .shown
            .clone()
            .ok_or_else(|| EngineError(format!("{tab} has no page to save yet")))?;
        self.serial += 1;
        let token = DownloadToken::new(self.serial);
        let Some(folder) = self.folder.as_ref() else {
            self.events
                .push(EngineEvent::Download(DownloadEvent::Refused {
                    token,
                    reason: "no downloads folder".to_owned(),
                }));
            return Ok(());
        };
        let file = folder.join(format!("{}.pdf", shown.title));
        self.files.insert(
            file.clone(),
            format!("%PDF-1.4 {}", shown.text).into_bytes(),
        );
        self.events
            .push(EngineEvent::Download(DownloadEvent::Saved {
                token,
                tab,
                jar,
                url,
                file,
            }));
        Ok(())
    }
}

/// The page that waits hears the answer and goes on: a confirm, a prompt, or a camera request
/// puts it in its title.
impl PageAsks for MemoryEngine {
    fn answer(&mut self, token: ReplyToken, answer: &Answer) -> bool {
        let Some((&tab, page)) = self
            .pages
            .iter_mut()
            .find(|(_, page)| page.asked.is_some_and(|(asked, _)| asked == token))
        else {
            return false;
        };
        let accepted = matches!(answer, Answer::Accept(_));
        let title = match page.asked.take().map(|(_, act)| act) {
            Some(Act::Confirm(_)) => format!("confirm:{accepted}"),
            Some(Act::Prompt(_, default)) => match answer {
                Answer::Accept(text) => format!("prompt:{}", text.as_deref().unwrap_or(default)),
                Answer::Decline => "prompt:null".to_owned(),
            },
            Some(Act::Camera) if accepted => "camera:allowed".to_owned(),
            Some(Act::Camera) => "camera:denied".to_owned(),
            _ => return true,
        };
        self.events.titled(tab, title);
        true
    }

    fn exit_fullscreen(&mut self, tab: TabId) {
        let Some(page) = self.pages.get_mut(&tab).filter(|page| page.fullscreen) else {
            return;
        };
        page.fullscreen = false;
        self.events.push(EngineEvent::Fullscreen {
            tab,
            state: Fullscreen::Left,
        });
        self.events.titled(tab, "fullscreen:none".to_owned());
    }
}

/// One page floats at a time, in a slot that draws nothing; playing or pausing a video the
/// page has is reported at once, and a page without one plays nothing.
impl PageVideo for MemoryEngine {
    fn float_video(&mut self, tab: TabId, _slot: &MemoryView) -> Result<(), EngineError> {
        self.page(tab)?;
        self.floated = Some(tab);
        Ok(())
    }

    fn dock_video(&mut self, tab: TabId) {
        if self.floated == Some(tab) {
            self.floated = None;
        }
    }

    fn play_video(&mut self, tab: TabId, play: bool) -> Result<(), EngineError> {
        let Some((_, audible)) = self.page(tab)?.video else {
            return Ok(());
        };
        let video = if play {
            Video::Playing { audible }
        } else {
            Video::Paused
        };
        self.play(tab, video);
        Ok(())
    }
}

/// A page keeps its zoom across its navigations; its viewport is as many CSS pixels narrower as
/// the zoom is larger.
impl PageZoom for MemoryEngine {
    fn set_zoom(&mut self, tab: TabId, level: ZoomLevel) {
        if let Some(page) = self.pages.get_mut(&tab) {
            page.zoom = level;
        }
    }
}
