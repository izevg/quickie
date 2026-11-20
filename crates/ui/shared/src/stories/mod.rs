//! Stories: named states of a widget, built from fixtures, rendered headless to PNG, compared
//! with reference images, and played with synthetic keys and clicks (`docs/quality.md`, UI
//! tests). Each UI crate lists its stories in a `stories` test target without the libtest
//! harness, so `AppKit` runs on the main thread, and hands them to [`run`].
//!
//! - [`compare`]: the perceptual comparison and its tolerance, pure Rust.
//! - [`render`]: the offscreen window, themes, the glass stand-in, PNG in and out.
//! - [`play`]: synthetic events and the accessibility tree.
//!
//! The command line of a story binary (`cargo xtask ui` runs it):
//! `[check | render | diff | accept | list] [<widget>[/<state>[.<theme>]] ...] [--scale <n>]
//! [--out <dir>] [--from <dir>] [--base <dir>]`. `check` is what `cargo test` runs. `diff`
//! renders like `render` and compares each image with the render of the same story in
//! `--base` (another commit's renders, made on this machine) instead of with its reference;
//! it lists the stories whose image changed in `<out>/changed.txt` (`cargo xtask ui diff`).

#![expect(
    clippy::print_stdout,
    reason = "a story binary reports to the terminal, like a test harness"
)]

mod compare;
mod play;
mod render;

use std::any::Any;
use std::cell::RefCell;
use std::fmt::{Debug, Display, Write as _};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use block2::StackBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSBitmapImageRep, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub use compare::{Pixels, TOLERANCE, Tolerance, Verdict, compare};
pub use play::{Key, Keystroke, Modifiers};
pub use render::{Theme, glass_stand_in, page_picture, renderer};

/// Light and dark: what most states are rendered in.
pub const LIGHT_DARK: &[Theme] = &[Theme::Light, Theme::Dark];
/// Light only: variants whose dark render adds nothing.
pub const LIGHT: &[Theme] = &[Theme::Light];
/// A browser window of a common laptop size.
pub const WINDOW: (f64, f64) = (1024.0, 640.0);

/// Room around the widget in a render, so its edge and ring show against the page.
const MARGIN: f64 = 24.0;

/// One named state of a widget.
#[derive(Clone, Copy)]
pub struct Story {
    /// The widget's group (`palette`, `tabs`, ... ; `settings` for the screen), which is also
    /// the first directory of its references.
    pub group: &'static str,
    /// The widget, in kebab case: `command-palette`.
    pub widget: &'static str,
    /// The state, in kebab case: `commands`, `no-matches`.
    pub state: &'static str,
    /// The themes it is rendered and compared in.
    pub themes: &'static [Theme],
    /// The window's content size in points.
    pub window: (f64, f64),
    /// Builds the state in the scene and plays it; checks go through [`Scene::check`].
    pub scene: fn(&mut Scene),
}

impl Story {
    /// `widget/state`.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}/{}", self.widget, self.state)
    }

    fn reference(&self, root: &Path, theme: Theme) -> PathBuf {
        root.join(self.group)
            .join(self.widget)
            .join(self.state)
            .join(format!("{}.png", theme.name()))
    }

    fn render_path(&self, out: &Path, theme: Theme, suffix: &str) -> PathBuf {
        out.join(self.widget)
            .join(format!("{}.{}{suffix}.png", self.state, theme.name()))
    }
}

/// Where a story builds its widget: an offscreen window whose content view stands for the
/// page, the theme it draws in, and the play tools.
pub struct Scene {
    mtm: MainThreadMarker,
    window: Retained<NSWindow>,
    host: Retained<NSView>,
    theme: Theme,
    kept: Vec<Box<dyn Any>>,
    route: Option<Box<dyn Fn(Keystroke) -> bool>>,
    failures: Vec<String>,
    crop: Option<NSRect>,
}

impl Scene {
    #[must_use]
    pub fn mtm(&self) -> MainThreadMarker {
        self.mtm
    }

    /// The window the widget is in.
    #[must_use]
    pub fn window(&self) -> &NSWindow {
        &self.window
    }

    /// The window's content view, where the browser puts its overlays.
    #[must_use]
    pub fn host(&self) -> &NSView {
        &self.host
    }

    #[must_use]
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Keeps `value` (the widget, its handlers) alive until the scene is rendered.
    pub fn keep(&mut self, value: impl Any) {
        self.kept.push(Box::new(value));
    }

    /// Stands in for the browser's key routing, which sees every key before the window does:
    /// `route` returns true for a key it handled, which then never reaches the widget.
    pub fn route(&mut self, route: impl Fn(Keystroke) -> bool + 'static) {
        self.route = Some(Box::new(route));
    }

    /// Presses `key` alone.
    pub fn press(&mut self, key: Key) {
        self.press_with(key, Modifiers::NONE);
    }

    /// Presses `key` with `modifiers`: the route first, then the window.
    pub fn press_with(&mut self, key: Key, modifiers: Modifiers) {
        let stroke = Keystroke { key, modifiers };
        if self.route.as_ref().is_some_and(|route| route(stroke)) {
            return;
        }
        play::send_key(&self.window, stroke);
    }

    /// Types `text` one key at a time, as a keyboard would.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let modifiers = if c.is_uppercase() {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            self.press_with(Key::Char(c), modifiers);
        }
    }

    /// The first visible view on the page whose accessibility title or label is `label`, to
    /// click it.
    #[must_use]
    pub fn view_labelled(&self, label: &str) -> Option<Retained<NSView>> {
        play::labelled(&self.host, label)
    }

    /// Clicks `view` in its middle.
    pub fn click(&mut self, view: &NSView) {
        let size = view.bounds().size;
        play::send_click(
            &self.window,
            view,
            NSPoint::new(size.width / 2.0, size.height / 2.0),
            self.mtm,
        );
    }

    /// Records a failed check unless `ok`.
    pub fn check(&mut self, ok: bool, what: impl Display) {
        if !ok {
            self.failures.push(what.to_string());
        }
    }

    /// Records a failed check unless `actual` is `expected`.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "stories compare what a getter just returned; owned values read best"
    )]
    pub fn check_eq<T: PartialEq + Debug>(&mut self, actual: T, expected: T, what: impl Display) {
        if actual != expected {
            self.failures
                .push(format!("{what}: got {actual:?}, expected {expected:?}"));
        }
    }

    /// The accessibility tree of the window's content, one element per line.
    #[must_use]
    pub fn accessibility(&self) -> String {
        play::accessibility(&self.host)
    }

    /// Crops the render to `view` (plus the margin) instead of to everything on the page.
    pub fn crop_to(&mut self, view: &NSView) {
        let frame = view.convertRect_toView(view.bounds(), Some(&self.host));
        self.crop = Some(frame);
    }
}

/// What a story binary was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Check,
    Render,
    Diff,
    Accept,
    List,
}

struct Options {
    mode: Mode,
    filters: Vec<String>,
    scale: f64,
    out: PathBuf,
    from: Option<PathBuf>,
    /// The renders `diff` compares with: another commit's, made on this machine.
    base: Option<PathBuf>,
    /// References are required and must come from this renderer (`QUICKIE_UI_STRICT=1`, CI).
    strict: bool,
}

/// Runs the story binary of the crate at `manifest_dir` (`env!("CARGO_MANIFEST_DIR")`) on
/// its `stories`; its references live in `<manifest_dir>/tests/snapshots`. True when every
/// story passed; the binary's `main` turns it into its exit code.
#[must_use]
pub fn run(stories: &[Story], manifest_dir: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        println!("stories: must run on the main thread (a test target with `harness = false`)");
        return false;
    };
    let manifest = Path::new(manifest_dir);
    let options = match options(std::env::args().skip(1), manifest) {
        Ok(options) => options,
        Err(error) => {
            println!("stories: {error}");
            return false;
        }
    };
    let chosen: Vec<(&Story, Theme)> = stories
        .iter()
        .flat_map(|story| story.themes.iter().map(move |&theme| (story, theme)))
        .filter(|(story, theme)| chosen(&options.filters, story, *theme))
        .collect();
    let references = manifest.join("tests").join("snapshots");
    let outcome = match options.mode {
        Mode::List => {
            list(stories, &options.filters);
            Ok(true)
        }
        // Another crate's stories were named.
        _ if chosen.is_empty() => Ok(true),
        Mode::Render | Mode::Check | Mode::Diff => {
            render::start(mtm);
            check(&chosen, &options, &references, mtm)
        }
        Mode::Accept => {
            render::start(mtm);
            accept(&chosen, &options, &references, mtm)
        }
    };
    outcome.unwrap_or_else(|error| {
        println!("stories: {error}");
        false
    })
}

fn options(args: impl Iterator<Item = String>, manifest: &Path) -> Result<Options, String> {
    let mut mode = Mode::Check;
    let mut filters = Vec::new();
    let mut scale = 1.0;
    let mut out = None;
    let mut from = None;
    let mut base = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "check" => mode = Mode::Check,
            "render" => mode = Mode::Render,
            "diff" => mode = Mode::Diff,
            "accept" => mode = Mode::Accept,
            "list" => mode = Mode::List,
            "--scale" => {
                scale = args
                    .next()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| (1.0..=4.0).contains(value))
                    .ok_or("--scale takes a number from 1 to 4")?;
            }
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out takes a directory")?)),
            "--from" => {
                from = Some(PathBuf::from(
                    args.next().ok_or("--from takes a directory")?,
                ));
            }
            "--base" => {
                base = Some(PathBuf::from(
                    args.next().ok_or("--base takes a directory")?,
                ));
            }
            // Flags a test runner may pass, such as `--nocapture`.
            flag if flag.starts_with("--") => {}
            filter => filters.push(filter.to_owned()),
        }
    }
    if mode != Mode::Render && (scale - 1.0).abs() > f64::EPSILON {
        return Err("references are 1x; --scale is for `render` only".to_owned());
    }
    if mode == Mode::Diff && base.is_none() {
        return Err("diff takes --base <dir>, the renders to compare with".to_owned());
    }
    Ok(Options {
        mode,
        filters,
        scale,
        out: out.unwrap_or_else(|| out_dir(manifest)),
        from,
        base,
        strict: std::env::var("QUICKIE_UI_STRICT").is_ok_and(|value| value == "1"),
    })
}

/// `$CARGO_TARGET_DIR/ui`, or `target/ui` in the workspace.
fn out_dir(manifest: &Path) -> PathBuf {
    if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(target).join("ui");
    }
    let root = manifest
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").exists())
        .unwrap_or(manifest);
    root.join("target").join("ui")
}

/// True when `story` in `theme` is one of `filters`: `widget`, `widget/state`, or
/// `widget/state.theme`; everything without filters.
fn chosen(filters: &[String], story: &Story, theme: Theme) -> bool {
    let id = story.id();
    filters.is_empty()
        || filters.iter().any(|filter| {
            let (path, wanted) = filter
                .split_once('.')
                .map_or((filter.as_str(), None), |(path, theme)| (path, Some(theme)));
            let path = path.trim_end_matches('/');
            (id == path || id.starts_with(&format!("{path}/")))
                && wanted.is_none_or(|wanted| wanted == theme.name())
        })
}

fn list(stories: &[Story], filters: &[String]) {
    for story in stories {
        if story
            .themes
            .iter()
            .any(|&theme| chosen(filters, story, theme))
        {
            let themes: Vec<&str> = story.themes.iter().map(|theme| theme.name()).collect();
            println!("{:<40} {}", story.id(), themes.join(" "));
        }
    }
}

/// One render of a story: the bitmap, the accessibility tree, the failed checks, the time.
struct Shot {
    image: Retained<NSBitmapImageRep>,
    accessibility: String,
    failures: Vec<String>,
    millis: u128,
}

fn shoot(story: &Story, theme: Theme, scale: f64, mtm: MainThreadMarker) -> Result<Shot, String> {
    let started = Instant::now();
    let size = NSSize::new(story.window.0, story.window.1);
    let window = render::window(size, theme, mtm);
    let host = window
        .contentView()
        .ok_or("the window has no content view")?;
    let scene = RefCell::new(Scene {
        mtm,
        window: window.clone(),
        host: host.clone(),
        theme,
        kept: Vec::new(),
        route: None,
        failures: Vec::new(),
        crop: None,
    });
    let image = RefCell::new(None);
    let build = StackBlock::new(|| {
        (story.scene)(&mut scene.borrow_mut());
        host.layoutSubtreeIfNeeded();
        let rect = scene.borrow().crop.map_or_else(|| content(&host), Some);
        let rect = rect.map_or_else(|| host.bounds(), |rect| around(rect, host.bounds()));
        *image.borrow_mut() = render::capture(&host, rect, scale);
    });
    match theme.appearance() {
        Some(appearance) => appearance.performAsCurrentDrawingAppearance(&build),
        None => {
            return Err(format!(
                "{}: no appearance for {}",
                story.id(),
                theme.name()
            ));
        }
    }
    let accessibility = play::accessibility(&host);
    let scene = scene.into_inner();
    let failures = scene.failures;
    drop(scene.kept);
    window.close();
    let image = image
        .into_inner()
        .ok_or_else(|| format!("{}: the render failed", story.id()))?;
    Ok(Shot {
        image,
        accessibility,
        failures,
        millis: started.elapsed().as_millis(),
    })
}

/// The frames of what the story put on the page, joined; scrims and other views as large as
/// the page do not count.
fn content(host: &NSView) -> Option<NSRect> {
    let bounds = host.bounds();
    host.subviews()
        .iter()
        .filter(|view| !view.isHidden())
        .map(|view| view.frame())
        .filter(|frame| {
            frame.size.width < bounds.size.width || frame.size.height < bounds.size.height
        })
        .reduce(union)
}

fn union(a: NSRect, b: NSRect) -> NSRect {
    let left = a.origin.x.min(b.origin.x);
    let bottom = a.origin.y.min(b.origin.y);
    let right = (a.origin.x + a.size.width).max(b.origin.x + b.size.width);
    let top = (a.origin.y + a.size.height).max(b.origin.y + b.size.height);
    NSRect::new(
        NSPoint::new(left, bottom),
        NSSize::new(right - left, top - bottom),
    )
}

/// `rect` grown by the margin, kept inside `bounds`, on whole points.
fn around(rect: NSRect, bounds: NSRect) -> NSRect {
    let left = (rect.origin.x - MARGIN).max(0.0).floor();
    let bottom = (rect.origin.y - MARGIN).max(0.0).floor();
    let right = (rect.origin.x + rect.size.width + MARGIN)
        .min(bounds.size.width)
        .ceil();
    let top = (rect.origin.y + rect.size.height + MARGIN)
        .min(bounds.size.height)
        .ceil();
    NSRect::new(
        NSPoint::new(left, bottom),
        NSSize::new((right - left).max(1.0), (top - bottom).max(1.0)),
    )
}

/// Renders every chosen story, runs its checks, and in `check` mode compares it with its
/// reference. Returns whether everything passed.
fn check(
    chosen: &[(&Story, Theme)],
    options: &Options,
    references: &Path,
    mtm: MainThreadMarker,
) -> Result<bool, String> {
    let ours = renderer();
    let theirs = fs::read_to_string(references.join(".renderer"))
        .map(|text| text.trim().to_owned())
        .ok();
    let compare_images = options.mode == Mode::Check && theirs.as_deref() == Some(&ours);
    let mut passed = true;
    if options.mode == Mode::Check && !compare_images {
        let from = theirs.as_deref().unwrap_or("nowhere (none accepted yet)");
        println!(
            "stories: references come from {from}, this is {ours}: images are rendered and \
             played, not compared"
        );
        passed &= !options.strict;
    }
    let started = Instant::now();
    let mut count = 0;
    let mut changed = String::new();
    for &(story, theme) in chosen {
        let shot = shoot(story, theme, options.scale, mtm)?;
        let path = story.render_path(&options.out, theme, "");
        render::write_png(&shot.image, &path)?;
        if theme == story.themes[0] {
            let tree = options
                .out
                .join(story.widget)
                .join(format!("{}.accessibility.txt", story.state));
            fs::write(&tree, &shot.accessibility)
                .map_err(|e| format!("{}: {e}", tree.display()))?;
        }
        let mut line = format!("{:>5} ms  {}.{}", shot.millis, story.id(), theme.name());
        let plain = line.len();
        if compare_images {
            let (ok, note) = verdict(story, theme, &path, references, options)?;
            passed &= ok;
            line.push_str(&note);
        } else if let Some(base) = options
            .base
            .as_deref()
            .filter(|_| options.mode == Mode::Diff)
        {
            // A story the base does not have is new; the caller sees that from the files.
            let before = story.render_path(base, theme, "");
            let diff_path = story.render_path(&options.out, theme, ".diff");
            if before.exists()
                && let Some(note) = differs(&before, &path, &diff_path)?
            {
                let _ = writeln!(changed, "{}.{}", story.id(), theme.name());
                line.push_str(&note);
            }
        } else if options.mode == Mode::Render {
            let _ = write!(line, "  {}", path.display());
        }
        for failure in &shot.failures {
            let _ = write!(line, "\n          play failed: {failure}");
            passed = false;
        }
        // `check` names only what needs a look; `render` lists every file it wrote.
        if options.mode == Mode::Render || line.len() > plain {
            println!("{line}");
        }
        count += 1;
    }
    fs::create_dir_all(&options.out).map_err(|e| format!("{}: {e}", options.out.display()))?;
    fs::write(options.out.join("renderer.txt"), format!("{ours}\n"))
        .map_err(|e| format!("renderer.txt: {e}"))?;
    if options.mode == Mode::Diff {
        fs::write(options.out.join("changed.txt"), changed)
            .map_err(|e| format!("changed.txt: {e}"))?;
    }
    println!(
        "stories: {count} render(s) in {} ms, {}",
        started.elapsed().as_millis(),
        if passed { "ok" } else { "FAILED" }
    );
    Ok(passed)
}

/// Compares the render at `actual` with its reference; writes the diff on a mismatch.
fn verdict(
    story: &Story,
    theme: Theme,
    actual: &Path,
    references: &Path,
    options: &Options,
) -> Result<(bool, String), String> {
    let reference = story.reference(references, theme);
    let diff_path = story.render_path(&options.out, theme, ".diff");
    // A diff from an earlier run would read as this run's.
    let _ = fs::remove_file(&diff_path);
    if !reference.exists() {
        let ok = !options.strict;
        return Ok((ok, "  no reference (cargo xtask ui accept)".to_owned()));
    }
    Ok(
        differs(&reference, actual, &diff_path)?
            .map_or((true, String::new()), |note| (false, note)),
    )
}

/// Compares the render at `actual` with the image at `expected` within [`TOLERANCE`]: `None`
/// when they match, else what differs. Writes the diff to `diff_path` when pixels differ.
fn differs(expected: &Path, actual: &Path, diff_path: &Path) -> Result<Option<String>, String> {
    let before = render::pixels(&*render::read_png(expected)?)
        .ok_or_else(|| format!("{}: unreadable", expected.display()))?;
    let rendered = render::pixels(&*render::read_png(actual)?)
        .ok_or_else(|| format!("{}: unreadable", actual.display()))?;
    Ok(match compare(&before, &rendered, &TOLERANCE) {
        Verdict::Same { .. } => None,
        Verdict::Resized { expected, actual } => Some(format!(
            "  CHANGED SIZE: {}x{} -> {}x{}",
            expected.0, expected.1, actual.0, actual.1
        )),
        Verdict::Different {
            differing,
            allowed,
            diff,
        } => {
            if let Some(rep) = render::rep_of(&diff) {
                render::write_png(&rep, diff_path)?;
            }
            Some(format!(
                "  DIFFERS: {differing} pixels (at most {allowed}), see {}",
                diff_path.display()
            ))
        }
    })
}

/// Writes references: from a directory of renders (`--from`, a CI artifact) or from fresh
/// renders here.
fn accept(
    chosen: &[(&Story, Theme)],
    options: &Options,
    references: &Path,
    mtm: MainThreadMarker,
) -> Result<bool, String> {
    let current = fs::read_to_string(references.join(".renderer"))
        .map(|text| text.trim().to_owned())
        .ok();
    let renderer = match &options.from {
        Some(from) => fs::read_to_string(from.join("renderer.txt"))
            .map(|text| text.trim().to_owned())
            .map_err(|e| format!("{}: {e}", from.join("renderer.txt").display()))?,
        None => renderer(),
    };
    if current.as_ref().is_some_and(|current| *current != renderer) && !options.filters.is_empty() {
        return Err(format!(
            "the references come from {}, these renders from {renderer}: accept every story \
             at once, or accept the CI renders with --from",
            current.unwrap_or_default()
        ));
    }
    for &(story, theme) in chosen {
        let target = story.reference(references, theme);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if let Some(from) = &options.from {
            let source = story.render_path(from, theme, "");
            fs::copy(&source, &target).map_err(|e| format!("{}: {e}", source.display()))?;
        } else {
            let shot = shoot(story, theme, 1.0, mtm)?;
            render::write_png(&shot.image, &target)?;
        }
        println!("accepted {}", target.display());
    }
    fs::write(references.join(".renderer"), format!("{renderer}\n"))
        .map_err(|e| format!(".renderer: {e}"))?;
    Ok(true)
}
