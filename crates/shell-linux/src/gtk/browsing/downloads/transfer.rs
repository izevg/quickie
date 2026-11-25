//! Tracks native downloads and chooses each destination off the GTK thread.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{CString, c_char, c_int, c_uint};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use quickie_domain_library::{Download, DownloadId, DownloadState};
use quickie_engine_webkit::download_name::{safe_name, unique_path};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use rustix::io::Errno;
use url::Url;

use crate::gtk::browsing::{notify, toasts::Toast};
use crate::gtk::ffi::{self, Bool, GError, Object};
use crate::gtk::{App, connect};

use super::policy::{copy_text, response_header};
use super::{Downloads, Source};

const MAX_TRANSFERS: usize = 16;
const DOWNLOAD_DIRECTORY: c_int = 2;
const SIGNAL_MATCH_DATA: c_uint = 1 << 4;

type Names = Arc<Mutex<HashSet<PathBuf>>>;
type Publication = Result<(), &'static str>;
type Destination = Result<Staged, &'static str>;

struct Staged {
    name: String,
    source: PathBuf,
    publish: Sender<Sender<Publication>>,
}

struct Reservation {
    path: PathBuf,
    names: Names,
}

impl Reservation {
    fn pick(folder: &Path, name: &str, names: &Names) -> Result<Self, &'static str> {
        let path = unique_path(folder, name, |path| {
            !matches!(fs::symlink_metadata(path), Err(error) if error.kind() == ErrorKind::NotFound)
                || !names
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(path.to_path_buf())
        })
        .ok_or("Cannot find an available name in Downloads")?;
        Ok(Self {
            path,
            names: Arc::clone(names),
        })
    }
}

fn stage(
    folder: &Path,
    name: &str,
    names: &Names,
    ready: &Sender<Destination>,
) -> Result<(), &'static str> {
    fs::create_dir_all(folder).map_err(|_| "Cannot create the Downloads folder")?;
    let reserved = Reservation::pick(folder, name, names)?;
    let name = reserved
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Cannot read the download name")?
        .to_owned();
    let private = tempfile::Builder::new()
        .prefix(".quickie-download-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(folder)
        .map_err(|_| "Cannot prepare the download destination")?;
    let source = private.path().join("payload");
    let (publish, requests) = mpsc::channel::<Sender<Publication>>();
    if ready
        .send(Ok(Staged {
            name,
            source: source.clone(),
            publish,
        }))
        .is_ok()
        && let Ok(complete) = requests.recv()
    {
        // WebKit's writer replaces a placeholder and may delete it on failure. Only this
        // atomic move touches the public name, even when another process claims it first.
        let result = renameat_with(CWD, &source, CWD, &reserved.path, RenameFlags::NOREPLACE)
            .map_err(|error| {
                if error == Errno::EXIST {
                    "That name is now in use in Downloads"
                } else {
                    "Cannot move the finished file to Downloads"
                }
            });
        drop(private);
        let _ = complete.send(result);
    }
    Ok(())
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.path);
    }
}

struct Live {
    native: Object,
    source: Source,
    url: Url,
    id: Option<DownloadId>,
    suggested: Option<String>,
    picking: Option<Receiver<Destination>>,
    staged: Option<Staged>,
}

impl Live {
    fn listed(&mut self, downloads: &Downloads) -> DownloadId {
        if let Some(id) = self.id {
            return id;
        }
        let name = self.suggested.take().unwrap_or_else(|| {
            safe_name(
                self.url
                    .path_segments()
                    .and_then(Iterator::last)
                    .unwrap_or_default(),
            )
        });
        let id = downloads.start(self.source, self.url.clone(), name);
        self.id = Some(id);
        id
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // SAFETY: Each live transfer owns one reference on the GTK thread.
        unsafe { ffi::g_object_unref(self.native) };
    }
}

#[derive(Default)]
pub(super) struct Transfers {
    live: RefCell<HashMap<Object, Live>>,
    publishing: RefCell<HashMap<DownloadId, Receiver<Publication>>>,
    workers: RefCell<Vec<JoinHandle<()>>>,
    reserved: Names,
    poll: Cell<Option<c_uint>>,
    data: Cell<Object>,
}

impl Transfers {
    pub(super) fn contains(&self, native: Object) -> bool {
        self.live.borrow().contains_key(&native)
    }

    pub(super) fn forget(&self, view: Object) {
        for live in self
            .live
            .borrow_mut()
            .values_mut()
            .filter(|live| live.source.view == view)
        {
            live.source.view = ptr::null_mut();
        }
    }

    fn has_capacity(&self) -> bool {
        self.live.borrow().len() + self.publishing.borrow().len() < MAX_TRANSFERS
    }

    pub(super) fn begin(&self, native: Object, source: Source, url: Url, data: Object) -> bool {
        if !self.has_capacity() {
            return false;
        }
        // SAFETY: WebKit supplies a live download on the GTK thread.
        unsafe { g_object_ref(native) };
        self.live.borrow_mut().insert(
            native,
            Live {
                native,
                source,
                url,
                id: None,
                suggested: None,
                picking: None,
                staged: None,
            },
        );
        connect(
            native,
            c"decide-destination",
            decide_destination as *const _,
            data,
        );
        connect(native, c"failed", failed as *const _, data);
        connect(native, c"finished", finished as *const _, data);
        self.data.set(data);
        if self.poll.get().is_none() {
            // SAFETY: This callback has the GLib timeout ABI. Detach removes it before App drops.
            self.poll
                .set(Some(unsafe { ffi::g_timeout_add(180, tick, data) }));
        }
        true
    }

    pub(super) fn retry(&self, app: &App, source: Source, url: &Url) -> bool {
        if !self.has_capacity() || !source.open(app) {
            return false;
        }
        let Ok(uri) = CString::new(url.as_str()) else {
            return false;
        };
        // SAFETY: The active ordinary view is live, and WebKit copies the URI.
        let native = unsafe { webkit_web_view_download_uri(source.view, uri.as_ptr()) };
        if native.is_null() {
            return false;
        }
        // The returned full reference is available before WebKit's asynchronous started signal.
        let accepted = self.begin(
            native,
            source,
            url.clone(),
            ptr::from_ref(app).cast_mut().cast(),
        );
        if !accepted {
            // SAFETY: The returned native download has not set a destination.
            unsafe { webkit_download_cancel(native) };
        }
        // SAFETY: This releases our constructor reference. The transfer owns a separate one.
        unsafe { ffi::g_object_unref(native) };
        accepted
    }

    pub(super) fn cancel(&self, downloads: &Downloads, id: DownloadId) {
        let transfer = self
            .live
            .borrow()
            .iter()
            .find_map(|(&native, live)| (live.id == Some(id)).then_some((native, live.source)));
        if let Some((native, source)) = transfer {
            let _ = downloads.list.borrow_mut().cancel(id);
            if !source.private() {
                downloads.save();
            }
            // SAFETY: The transfer owns a reference until WebKit reports finished.
            unsafe { webkit_download_cancel(native) };
        }
    }

    fn collect_publications(&self, downloads: &Downloads, ended: impl Fn(&Download)) {
        let mut changed = false;
        self.publishing.borrow_mut().retain(|&id, results| {
            let result = match results.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => {
                    Err("Cannot move the finished file to Downloads")
                }
            };
            let mut list = downloads.list.borrow_mut();
            let moved = match result {
                Ok(()) => list.finish(id),
                Err(reason) => list.fail(id, reason.to_owned()),
            };
            if moved.is_ok()
                && let Some(download) = list.get(id)
            {
                ended(download);
            }
            changed |= !downloads
                .origins
                .borrow()
                .get(&id)
                .copied()
                .is_some_and(Source::private);
            false
        });
        if changed {
            downloads.save();
        }
    }

    pub(super) fn detach(&self, downloads: &Downloads) {
        if let Some(source) = self.poll.take() {
            // SAFETY: This source belongs to the live GTK main context.
            unsafe { ffi::g_source_remove(source) };
        }
        for (native, _transfer) in self.live.take() {
            // SAFETY: The transfer keeps this download alive. Disconnect App callbacks before
            // cancellation, since the event loop has returned and its App is about to be dropped.
            unsafe {
                g_signal_handlers_disconnect_matched(
                    native,
                    SIGNAL_MATCH_DATA,
                    0,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    self.data.get(),
                )
            };
            // SAFETY: The download is still live through cancellation.
            unsafe { webkit_download_cancel(native) };
        }
        for worker in self.workers.take() {
            let _ = worker.join();
        }
        // A worker may have published a file since the last tick. Record its result before
        // browsing flushes the store, so a completed file never comes back as interrupted.
        self.collect_publications(downloads, |_| {});
    }
}

pub(super) fn folder() -> Option<PathBuf> {
    // SAFETY: GLib owns the returned path for the duration of this GTK application.
    let path = unsafe { g_get_user_special_dir(DOWNLOAD_DIRECTORY) };
    let configured = copy_text(path)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    configured.or_else(|| {
        env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|home| home.join("Downloads"))
    })
}

extern "C" fn decide_destination(native: Object, name: *const c_char, data: Object) -> Bool {
    // SAFETY: The native signal carries the live GTK App reference.
    let app = unsafe { &*data.cast::<App>() };
    let downloads = &app.browsing.downloads;
    let Some(name) = copy_text(name).map(|name| safe_name(&name)) else {
        reject(app, native, "Cannot read the download name");
        return ffi::TRUE;
    };
    let Some(folder) = downloads.folder.get().cloned() else {
        reject(app, native, "Cannot locate the Downloads folder");
        return ffi::TRUE;
    };
    // SAFETY: The live download owns its response when it asks for a destination.
    let response = unsafe { webkit_download_get_response(native) };
    if !response.is_null() {
        // SAFETY: The response is borrowed from the live download.
        let status = unsafe { webkit_uri_response_get_status_code(response) };
        if !(200..300).contains(&status) {
            reject(app, native, "The server refused the download");
            return ffi::TRUE;
        }
    }
    let (sender, receiver) = mpsc::channel();
    let names = Arc::clone(&downloads.transfers.reserved);
    let suggested = name.clone();
    let worker = thread::Builder::new()
        .name("quickie-download-path".to_owned())
        .spawn(move || {
            if let Err(error) = stage(&folder, &name, &names, &sender) {
                let _ = sender.send(Err(error));
            }
        });
    match worker {
        Ok(worker) => {
            let mut workers = downloads.transfers.workers.borrow_mut();
            workers.retain(|worker| !worker.is_finished());
            workers.push(worker);
            if let Some(transfer) = downloads.transfers.live.borrow_mut().get_mut(&native) {
                transfer.suggested = Some(suggested);
                transfer.picking = Some(receiver);
            }
        }
        Err(_) => reject(app, native, "Cannot prepare the download destination"),
    }
    ffi::TRUE
}

fn reject(app: &App, native: Object, reason: &str) {
    mark_failed(app, native, reason);
    // SAFETY: The retained download is live and may safely cancel its pending destination.
    unsafe { webkit_download_cancel(native) };
}

extern "C" fn tick(data: Object) -> Bool {
    // SAFETY: The timer is removed when the GTK App shuts down.
    let app = unsafe { &*data.cast::<App>() };
    let downloads = &app.browsing.downloads;
    let transfers = &downloads.transfers;
    let mut destinations = Vec::new();
    {
        let mut live = transfers.live.borrow_mut();
        if live.is_empty() && transfers.publishing.borrow().is_empty() {
            transfers.poll.set(None);
            return ffi::FALSE;
        }
        for (&native, transfer) in live.iter_mut() {
            if let Some(receiver) = &transfer.picking {
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => {
                        Err("Cannot prepare the download destination")
                    }
                };
                transfer.picking = None;
                destinations.push((native, result));
            } else if let Some(id) = transfer.id {
                update_progress(downloads, native, id);
            }
        }
    }
    for (native, destination) in destinations {
        match destination {
            Ok(staged) => install(app, native, staged),
            Err(reason) => reject(app, native, reason),
        }
    }
    transfers.collect_publications(downloads, |download| notify_ended(app, download));
    downloads.refresh();
    ffi::TRUE
}

fn install(app: &App, native: Object, staged: Staged) {
    let downloads = &app.browsing.downloads;
    let Some(path) = staged
        .source
        .to_str()
        .and_then(|path| CString::new(path).ok())
    else {
        reject(app, native, "Unsupported path to Downloads");
        return;
    };
    {
        let mut live = downloads.transfers.live.borrow_mut();
        let Some(transfer) = live.get_mut(&native) else {
            return;
        };
        transfer.suggested = Some(staged.name.clone());
        let id = transfer.listed(downloads);
        transfer.staged = Some(staged);
        if let Some(download) = downloads.list.borrow().get(id) {
            notify(
                app,
                Toast::info("Downloading").with_detail(&download.file_name),
            );
        }
    }
    // SAFETY: The retained WebKit download has an outstanding destination decision.
    unsafe { webkit_download_set_allow_overwrite(native, ffi::FALSE) };
    // SAFETY: WebKitGTK 6.0 copies the NUL-terminated UTF-8 absolute filesystem path.
    unsafe { webkit_download_set_destination(native, path.as_ptr()) };
}

fn update_progress(downloads: &Downloads, native: Object, id: DownloadId) {
    let (_, received, total) = progress(native);
    let _ = downloads.list.borrow_mut().progress(id, received, total);
}

fn progress(native: Object) -> (Object, u64, Option<u64>) {
    // SAFETY: The transfer owns a reference to this native download during the tick.
    let received = unsafe { webkit_download_get_received_data_length(native) };
    // SAFETY: The download owns its response, which may not have arrived yet.
    let response = unsafe { webkit_download_get_response(native) };
    let total = if response.is_null() {
        0
    } else {
        // SAFETY: The response is live while the retained download is borrowed.
        unsafe { webkit_uri_response_get_content_length(response) }
    };
    (response, received, (total != 0).then_some(total))
}

extern "C" fn failed(native: Object, _: *const GError, data: Object) {
    // SAFETY: The native signal carries the live App registered for this transfer.
    let app = unsafe { &*data.cast::<App>() };
    mark_failed(app, native, "The download could not be completed");
}

fn mark_failed(app: &App, native: Object, reason: &str) {
    let downloads = &app.browsing.downloads;
    if let Some(transfer) = downloads.transfers.live.borrow_mut().get_mut(&native) {
        let id = transfer.listed(downloads);
        {
            let mut list = downloads.list.borrow_mut();
            if list.fail(id, reason.to_owned()).is_ok()
                && let Some(download) = list.get(id)
            {
                notify_ended(app, download);
            }
        }
        if !transfer.source.private() {
            downloads.save();
        }
    }
    downloads.refresh();
}

fn notify_ended(app: &App, download: &Download) {
    let toast = match download.state {
        DownloadState::Finished => Toast::success("Downloaded"),
        DownloadState::Failed(_) => Toast::problem("Download failed"),
        _ => return,
    };
    notify(app, toast.with_detail(&download.file_name));
}

extern "C" fn finished(native: Object, data: Object) {
    // SAFETY: The native signal carries the live App registered for this transfer.
    let app = unsafe { &*data.cast::<App>() };
    let downloads = &app.browsing.downloads;
    let Some(mut transfer) = downloads.transfers.live.borrow_mut().remove(&native) else {
        return;
    };
    let id = transfer.listed(downloads);
    let mut list = downloads.list.borrow_mut();
    if list
        .get(id)
        .is_some_and(|download| matches!(download.state, DownloadState::InProgress { .. }))
    {
        let (complete, result) = mpsc::channel();
        let (response, received, total) = progress(native);
        let incomplete = total.is_some_and(|total| received < total)
            && response_header(response, c"Content-Encoding")
                .is_none_or(|encoding| encoding.eq_ignore_ascii_case("identity"));
        if incomplete {
            let _ = list.fail(id, "The download ended before all bytes arrived".to_owned());
        } else if transfer
            .staged
            .take()
            .is_some_and(|staged| staged.publish.send(complete).is_ok())
        {
            downloads
                .transfers
                .publishing
                .borrow_mut()
                .insert(id, result);
        } else {
            let _ = list.fail(id, "The download did not create a file".to_owned());
        }
        if let Some(download) = list.get(id) {
            notify_ended(app, download);
        }
    }
    drop(list);
    if !transfer.source.private() {
        downloads.save();
    }
    downloads.refresh();
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_get_user_special_dir(directory: c_int) -> *const c_char;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_ref(object: Object) -> Object;
    fn g_signal_handlers_disconnect_matched(
        instance: Object,
        mask: c_uint,
        signal_id: c_uint,
        detail: c_uint,
        closure: Object,
        function: Object,
        data: Object,
    ) -> c_uint;
}

#[link(name = "webkitgtk-6.0")]
unsafe extern "C" {
    fn webkit_web_view_download_uri(view: Object, uri: *const c_char) -> Object;
    fn webkit_download_set_allow_overwrite(download: Object, allow: Bool);
    fn webkit_download_set_destination(download: Object, path: *const c_char);
    fn webkit_download_get_response(download: Object) -> Object;
    fn webkit_download_get_received_data_length(download: Object) -> u64;
    fn webkit_uri_response_get_content_length(response: Object) -> u64;
    fn webkit_uri_response_get_status_code(response: Object) -> c_uint;
    fn webkit_download_cancel(download: Object);
}
