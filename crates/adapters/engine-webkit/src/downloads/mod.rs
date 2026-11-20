//! Downloads on `WKDownload`. A page hands over a navigation that became a download (a
//! response `WebKit` cannot show, an attachment, a link with `download`); the center becomes
//! its delegate, picks the file it goes to, watches its progress, and reports
//! [`DownloadReport`]s for the shell to drain.
//!
//! **Never overwrite.** A download goes to the Downloads folder under its suggested name made
//! safe ([`name::safe_name`]); when that name is taken on disk, or by a download still running,
//! it gets the next free number, as the Finder numbers copies. Looking at the disk is I/O, so the
//! name is picked on a worker thread and `WebKit` hears it back on the main thread. A cancelled
//! or failed download leaves a truncated file that looks whole, so it is deleted (it did not
//! exist before the download), and its name stays taken until it is gone.
//!
//! Progress is polled four times a second while something downloads, from `WKDownload`'s
//! `NSProgress`, and reported only when it moved.

mod delegate;
mod name;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use block2::RcBlock;
use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, Message};
use objc2_foundation::{NSData, NSProgressReporting, NSString, NSTimer, NSURL, NSURLRequest};
use objc2_web_kit::{WKDownload, WKWebView};
use quickie_domain_browsing::{Jar, TabId};
use url::Url;

use delegate::{DelegateEvent, DownloadDelegate};
pub(crate) use name::{pdf_name, wants_download};

/// How often a running download's progress is read, in seconds.
const POLL_INTERVAL: f64 = 0.25;

/// The engine's handle on one download, for as long as Quickie runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DownloadKey(u64);

/// What a download did since the shell last asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadReport {
    /// It has a file and is fetching into it.
    Started {
        key: DownloadKey,
        /// The tab whose page started it, and that page's jar.
        tab: TabId,
        jar: Jar,
        /// Where the bytes come from.
        url: Url,
        /// The file it is saved to; it did not exist before.
        file: PathBuf,
    },
    /// Bytes arrived; `total` is the size the server announced, if it did.
    Progress {
        key: DownloadKey,
        received: u64,
        total: Option<u64>,
    },
    /// Every byte is in the file.
    Finished { key: DownloadKey },
    /// It stopped, for `reason`.
    Failed { key: DownloadKey, reason: String },
    /// It never started: no file could be picked for it, for `reason`.
    Refused { key: DownloadKey, reason: String },
    /// A file Quickie made from a page, such as a PDF of it, is written whole: it starts and
    /// ends at once.
    Saved {
        key: DownloadKey,
        /// The tab whose page it was made from, and that page's jar.
        tab: TabId,
        jar: Jar,
        /// The page.
        url: Url,
        /// The file; it did not exist before.
        file: PathBuf,
    },
}

impl DownloadReport {
    /// The download it is about.
    #[must_use]
    pub fn key(&self) -> DownloadKey {
        match self {
            Self::Started { key, .. }
            | Self::Progress { key, .. }
            | Self::Finished { key }
            | Self::Failed { key, .. }
            | Self::Refused { key, .. }
            | Self::Saved { key, .. } => *key,
        }
    }
}

/// A download `WebKit` is running.
struct Live {
    key: DownloadKey,
    download: Retained<WKDownload>,
    tab: TabId,
    jar: Jar,
    /// Set once its destination is picked; it is reported from then on.
    file: Option<PathBuf>,
    /// The progress reported last.
    reported: (u64, Option<u64>),
}

/// Every running download, and what they reported.
pub(crate) struct DownloadCenter {
    mtm: MainThreadMarker,
    delegate: Retained<DownloadDelegate>,
    directory: RefCell<Option<PathBuf>>,
    /// Files picked for running downloads: taken even before `WebKit` creates them.
    reserved: Arc<Mutex<HashSet<PathBuf>>>,
    live: RefCell<Vec<Live>>,
    next: Cell<u64>,
    reports: RefCell<Vec<DownloadReport>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    /// Tells the engine's inbox that reports wait.
    notify: Box<dyn Fn()>,
}

impl DownloadCenter {
    /// A center with no downloads; `notify` runs whenever a report is recorded.
    pub(crate) fn new(notify: impl Fn() + 'static, mtm: MainThreadMarker) -> Rc<Self> {
        Rc::new_cyclic(|this: &Weak<Self>| {
            let this = this.clone();
            let delegate = DownloadDelegate::new(
                Box::new(move |event| {
                    if let Some(center) = this.upgrade() {
                        center.delegate_event(event);
                    }
                }),
                mtm,
            );
            Self {
                mtm,
                delegate,
                directory: RefCell::new(None),
                reserved: Arc::default(),
                live: RefCell::new(Vec::new()),
                next: Cell::new(0),
                reports: RefCell::new(Vec::new()),
                timer: RefCell::new(None),
                notify: Box::new(notify),
            }
        })
    }

    /// Where downloads are saved from now on; created when the first one arrives.
    pub(crate) fn set_directory(&self, directory: PathBuf) {
        *self.directory.borrow_mut() = Some(directory);
    }

    /// Takes over a download a page started: the center becomes its delegate.
    pub(crate) fn adopt(&self, tab: TabId, jar: Jar, download: &WKDownload) {
        let key = DownloadKey(self.next.get());
        self.next.set(key.0 + 1);
        // SAFETY: the delegate is weak; the center keeps it alive for as long as it lives, and
        // a download that outlives the center calls a delegate whose center is gone, which
        // ignores it.
        unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate))) };
        self.live.borrow_mut().push(Live {
            key,
            download: download.retain(),
            tab,
            jar,
            file: None,
            reported: (0, None),
        });
    }

    /// Fetches `url` to a file from `view`'s page, in its jar: a retry.
    pub(crate) fn start(this: &Rc<Self>, view: &WKWebView, url: &Url, tab: TabId, jar: Jar) {
        let Some(parsed) = NSURL::URLWithString(&NSString::from_str(url.as_str())) else {
            return;
        };
        let request = NSURLRequest::requestWithURL(&parsed);
        let center = Rc::downgrade(this);
        let adopt = RcBlock::new(move |download: NonNull<WKDownload>| {
            if let Some(center) = center.upgrade() {
                // SAFETY: WebKit passes a valid download that outlives this call.
                center.adopt(tab, jar, unsafe { download.as_ref() });
            }
        });
        // SAFETY: `request` is a valid request for an absolute URL, and WebKit calls the block on
        // the main thread, where the captured center lives.
        unsafe { view.startDownloadUsingRequest_completionHandler(&request, &adopt) };
    }

    /// Writes `made`, the bytes of a file made from `tab`'s page at `url`, into the folder
    /// under `name` or its next free number, on a worker thread; reports it
    /// [`DownloadReport::Saved`] on the main thread, or [`DownloadReport::Refused`] with why it
    /// was not made or written.
    pub(crate) fn save(
        this: &Rc<Self>,
        tab: TabId,
        jar: Jar,
        url: Url,
        name: &str,
        made: Result<Vec<u8>, String>,
    ) {
        let key = DownloadKey(this.next.get());
        this.next.set(key.0 + 1);
        let directory = this.directory.borrow().clone();
        let (bytes, directory) = match (made, directory) {
            (Ok(bytes), Some(directory)) => (bytes, directory),
            (Err(reason), _) => return this.report(DownloadReport::Refused { key, reason }),
            (Ok(_), None) => {
                let reason = "there is no downloads folder".to_owned();
                return this.report(DownloadReport::Refused { key, reason });
            }
        };
        let name = name::safe_name(name);
        let reserved = Arc::clone(&this.reserved);
        let center = MainThreadBound::new(Rc::downgrade(this), this.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-save-file".to_owned())
            .spawn(move || {
                let written = write_new(&directory, &name, &bytes, &reserved);
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    let Some(center) = center.into_inner(mtm).upgrade() else {
                        return;
                    };
                    center.report(match written {
                        Ok(file) => DownloadReport::Saved {
                            key,
                            tab,
                            jar,
                            url,
                            file,
                        },
                        Err(reason) => DownloadReport::Refused { key, reason },
                    });
                });
            });
        if spawned.is_err() {
            this.report(DownloadReport::Refused {
                key,
                reason: "Quickie could not start a thread to write the file".to_owned(),
            });
        }
    }

    /// Stops a running download and deletes the part of the file it wrote; nothing more is
    /// reported for it.
    pub(crate) fn cancel(&self, key: DownloadKey) {
        let Some(live) = self.take(|live| live.key == key) else {
            return;
        };
        let reserved = Arc::clone(&self.reserved);
        let file = live.file;
        let stopped = RcBlock::new(move |_resume: *mut NSData| {
            if let Some(file) = &file {
                discard(file.clone(), Arc::clone(&reserved));
            }
        });
        // SAFETY: WebKit calls the block once the download stopped, on the main thread; it
        // captures only `Send` values.
        unsafe { live.download.cancel(Some(&stopped)) };
    }

    /// The reports since the last call, oldest first.
    pub(crate) fn take_reports(&self) -> Vec<DownloadReport> {
        std::mem::take(&mut *self.reports.borrow_mut())
    }

    fn delegate_event(self: Rc<Self>, event: DelegateEvent) {
        match event {
            DelegateEvent::Destination {
                download,
                url,
                suggested,
                decide,
            } => self.choose_file(&download, url, &suggested, decide),
            DelegateEvent::Finished(download) => {
                self.end(&download, false, |key| DownloadReport::Finished { key });
            }
            DelegateEvent::Failed(download, reason) => {
                self.end(&download, true, |key| DownloadReport::Failed {
                    key,
                    reason,
                });
            }
        }
    }

    /// Picks the file on a worker thread, then answers `WebKit` and reports the start on the
    /// main thread.
    fn choose_file(
        self: Rc<Self>,
        download: &WKDownload,
        url: Option<String>,
        suggested: &str,
        decide: RcBlock<dyn Fn(*mut NSURL)>,
    ) {
        let key = self
            .live
            .borrow()
            .iter()
            .find(|live| *live.download == *download)
            .map(|live| live.key);
        let (Some(key), Some(directory)) = (key, self.directory.borrow().clone()) else {
            decide.call((std::ptr::null_mut(),));
            return;
        };
        let url = url
            .and_then(|url| Url::parse(&url).ok())
            .or_else(|| Url::parse("about:blank").ok());
        let name = name::safe_name(suggested);
        let reserved = Arc::clone(&self.reserved);
        let answer = MainThreadBound::new((Rc::downgrade(&self), decide), self.mtm);
        let spawned = thread::Builder::new()
            .name("quickie-download-name".to_owned())
            .spawn(move || {
                let file = pick(&directory, &name, &reserved);
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    let (center, decide) = answer.into_inner(mtm);
                    match (center.upgrade(), file, url) {
                        (Some(center), Ok(file), Some(url)) => {
                            Self::started(&center, key, url, file, &decide);
                        }
                        (center, file, _) => {
                            decide.call((std::ptr::null_mut(),));
                            if let Some(center) = center {
                                let reason = file.err().unwrap_or_default();
                                center.refused(key, &reason);
                            }
                        }
                    }
                });
            });
        if spawned.is_err() {
            self.refused(key, "Quickie could not start a thread to name the file");
        }
    }

    /// The file is picked: `WebKit` writes there from now on.
    fn started(
        this: &Rc<Self>,
        key: DownloadKey,
        url: Url,
        file: PathBuf,
        decide: &RcBlock<dyn Fn(*mut NSURL)>,
    ) {
        let started = {
            let mut live = this.live.borrow_mut();
            live.iter_mut().find(|live| live.key == key).map(|live| {
                live.file = Some(file.clone());
                (live.tab, live.jar)
            })
        };
        let Some((tab, jar)) = started else {
            // Cancelled while the name was being picked.
            this.unreserve(&file);
            decide.call((std::ptr::null_mut(),));
            return;
        };
        let target =
            NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&file.to_string_lossy()), false);
        decide.call((Retained::as_ptr(&target).cast_mut(),));
        this.report(DownloadReport::Started {
            key,
            tab,
            jar,
            url,
            file,
        });
        Self::poll_while_running(this);
    }

    /// No file could be picked: the download stops before it started.
    fn refused(&self, key: DownloadKey, reason: &str) {
        if let Some(live) = self.take(|live| live.key == key) {
            // SAFETY: cancelling has no preconditions.
            unsafe { live.download.cancel(None) };
            self.report(DownloadReport::Refused {
                key,
                reason: reason.to_owned(),
            });
        }
    }

    /// A download finished, or `failed`, as `report` says; a failed one's part file is deleted.
    fn end(
        &self,
        download: &WKDownload,
        failed: bool,
        report: impl FnOnce(DownloadKey) -> DownloadReport,
    ) {
        let Some(live) = self.take(|live| *live.download == *download) else {
            return;
        };
        let Some(file) = live.file else {
            return;
        };
        if failed {
            discard(file, Arc::clone(&self.reserved));
        } else {
            self.unreserve(&file);
        }
        self.report(report(live.key));
    }

    /// Removes the running download `which` matches. Its file name stays reserved until the
    /// caller releases it.
    fn take(&self, which: impl Fn(&Live) -> bool) -> Option<Live> {
        let mut live = self.live.borrow_mut();
        let index = live.iter().position(which)?;
        Some(live.remove(index))
    }

    fn unreserve(&self, file: &Path) {
        self.reserved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(file);
    }

    fn report(&self, report: DownloadReport) {
        self.reports.borrow_mut().push(report);
        (self.notify)();
    }

    /// Reads every running download's progress, reporting what moved; stops the timer once
    /// nothing runs.
    fn poll(&self) {
        let moved: Vec<DownloadReport> = self
            .live
            .borrow_mut()
            .iter_mut()
            .filter(|live| live.file.is_some())
            .filter_map(|live| {
                let progress = live.download.progress();
                let received = u64::try_from(progress.completedUnitCount()).unwrap_or(0);
                let total = u64::try_from(progress.totalUnitCount()).ok();
                let now = (received, total);
                (now != live.reported).then(|| {
                    live.reported = now;
                    DownloadReport::Progress {
                        key: live.key,
                        received,
                        total,
                    }
                })
            })
            .collect();
        for report in moved {
            self.report(report);
        }
        if self.live.borrow().is_empty()
            && let Some(timer) = self.timer.borrow_mut().take()
        {
            timer.invalidate();
        }
    }

    fn poll_while_running(this: &Rc<Self>) {
        if this.timer.borrow().is_some() {
            return;
        }
        let center = Rc::downgrade(this);
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(center) = center.upgrade() {
                center.poll();
            }
        });
        // SAFETY: a repeating timer on the main run loop, which retains it until `poll`
        // invalidates it; the block runs on the main thread that owns what it captures.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(POLL_INTERVAL, true, &block)
        };
        *this.timer.borrow_mut() = Some(timer);
    }
}

/// Deletes the part of a file a download wrote before it stopped, on a worker thread, and only
/// then releases its name. The file did not exist before the download picked it, so nothing
/// else is lost.
fn discard(file: PathBuf, reserved: Arc<Mutex<HashSet<PathBuf>>>) {
    let release = move || {
        // A part file WebKit already removed, or never wrote, is no error.
        let _ = std::fs::remove_file(&file);
        reserved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&file);
    };
    // Without a thread the name stays reserved, so nothing can overwrite the part file.
    let _ = thread::Builder::new()
        .name("quickie-download-discard".to_owned())
        .spawn(release);
}

/// Writes `bytes` to the first free file for `name` in `directory` and returns it. The file is
/// created new, never over another; one written only in part is deleted.
fn write_new(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    reserved: &Mutex<HashSet<PathBuf>>,
) -> Result<PathBuf, String> {
    let file = pick(directory, name, reserved)?;
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .and_then(|mut opened| opened.write_all(bytes));
    if written.is_err() {
        // Nothing was there before `create_new`, so nothing else is lost.
        let _ = std::fs::remove_file(&file);
    }
    reserved
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&file);
    written
        .map(|()| file.clone())
        .map_err(|error| format!("{}: {error}", file.display()))
}

/// The first free file for `name` in `directory`, reserved; the directory is created first.
fn pick(
    directory: &Path,
    name: &str,
    reserved: &Mutex<HashSet<PathBuf>>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut reserved = reserved.lock().unwrap_or_else(PoisonError::into_inner);
    let file = name::unique_path(directory, name, |path| {
        reserved.contains(path) || path.symlink_metadata().is_ok()
    })
    .ok_or_else(|| format!("no free name for {name} in {}", directory.display()))?;
    reserved.insert(file.clone());
    Ok(file)
}
