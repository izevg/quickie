//! Headless rendering: an offscreen window per scene, the theme as an `NSAppearance`, the
//! deterministic glass stand-in, and PNG in and out through `NSBitmapImageRep`.
//!
//! Nothing here needs a display: the window is never ordered in, and a view draws into a
//! bitmap with `cacheDisplayInRect:toBitmapImageRep:`, the way `AppKit` prints a view.

use std::cell::Cell;
use std::fs;
use std::path::Path;

use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAccessibilityHighContrastAqua,
    NSAppearanceNameAccessibilityHighContrastDarkAqua, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType,
    NSBitmapImageFileType, NSBitmapImageRep, NSCalibratedRGBColorSpace, NSColor, NSColorSpace,
    NSGraphicsContext, NSImage, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSData, NSDictionary, NSPoint, NSProcessInfo, NSRect, NSSize};

use super::compare::Pixels;

thread_local! {
    static STAND_IN: Cell<bool> = const { Cell::new(false) };
}

/// True while a story renders: glass surfaces are opaque stand-ins
/// ([`crate::glass::GlassSurface`]), so renders do not depend on what is behind the window or
/// on whether the system can draw Liquid Glass offscreen (it cannot: glass renders empty, and
/// translucent content inside it draws twice).
#[must_use]
pub fn glass_stand_in() -> bool {
    STAND_IN.with(Cell::get)
}

/// How a scene looks: the appearance and the contrast setting it is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
    /// Light with Increase Contrast.
    Contrast,
    /// Dark with Increase Contrast.
    ContrastDark,
}

impl Theme {
    pub const ALL: [Self; 4] = [Self::Light, Self::Dark, Self::Contrast, Self::ContrastDark];

    /// The file name stem of a render in this theme.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Contrast => "contrast",
            Self::ContrastDark => "contrast-dark",
        }
    }

    /// The page under the widget: a fixed mid grey, lighter in light themes, so the edge of
    /// any surface shows against it on every system.
    fn page(self) -> Retained<NSColor> {
        let grey = match self {
            Self::Light | Self::Contrast => 0.74,
            Self::Dark | Self::ContrastDark => 0.30,
        };
        NSColor::colorWithSRGBRed_green_blue_alpha(grey, grey, grey + 0.02, 1.0)
    }

    pub(super) fn appearance(self) -> Option<Retained<NSAppearance>> {
        // SAFETY: the appearance names are immutable strings exported by AppKit.
        let name = unsafe {
            match self {
                Self::Light => NSAppearanceNameAqua,
                Self::Dark => NSAppearanceNameDarkAqua,
                Self::Contrast => NSAppearanceNameAccessibilityHighContrastAqua,
                Self::ContrastDark => NSAppearanceNameAccessibilityHighContrastDarkAqua,
            }
        };
        NSAppearance::appearanceNamed(name)
    }
}

/// Makes this process a headless `AppKit` app: no Dock icon, no menu bar, never active. Glass
/// surfaces built from now on are stand-ins.
pub(super) fn start(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    app.finishLaunching();
    STAND_IN.with(|on| on.set(true));
}

/// The renderer a reference was made with: the macOS major version. Text rasterizes the same
/// within one, and differently across them.
#[must_use]
pub fn renderer() -> String {
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    format!("macos-{}", version.majorVersion)
}

/// An offscreen, borderless window of `size` whose content view is the page under the widget,
/// a plain grey backdrop ([`Theme::page`]).
pub(super) fn window(size: NSSize, theme: Theme, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::new(-20_000.0, -20_000.0), size);
    // SAFETY: a fresh allocation; a borderless, buffered, non-deferred window is always valid.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: the scene owns the window; AppKit must not release it on close as well.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setAppearance(theme.appearance().as_deref());
    let host = NSView::new(mtm);
    host.setWantsLayer(true);
    window.setContentView(Some(&host));
    host.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
    if let Some(layer) = host.layer() {
        layer.setBackgroundColor(Some(&theme.page().CGColor()));
    }
    window
}

/// Draws `rect` of `view` (in the view's coordinates) at `scale` pixels per point.
pub(super) fn capture(
    view: &NSView,
    rect: NSRect,
    scale: f64,
) -> Option<Retained<NSBitmapImageRep>> {
    let rep = blank(rect.size, scale)?;
    view.cacheDisplayInRect_toBitmapImageRep(rect, &rep);
    Some(rep)
}

/// An sRGB bitmap of `size` points at `scale`, all clear: 8 bits per sample, RGBA, alpha
/// premultiplied and last.
fn blank(size: NSSize, scale: f64) -> Option<Retained<NSBitmapImageRep>> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a window's size in pixels, rounded, fits an NSInteger"
    )]
    let pixels = |points: f64| (points * scale).round().max(1.0) as isize;
    // SAFETY: null planes ask the rep to allocate its own buffer; the colour space name is an
    // immutable string exported by AppKit, and 0 bytes per row / bits per pixel mean "packed".
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            pixels(size.width),
            pixels(size.height),
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            0,
            0,
        )
    }?;
    let rep = rep.bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())?;
    rep.setSize(size);
    Some(rep)
}

/// The pixels of `rep`, redrawn into the one layout every comparison reads.
pub(super) fn pixels(rep: &NSBitmapImageRep) -> Option<Pixels> {
    let width = usize::try_from(rep.pixelsWide()).ok()?;
    let height = usize::try_from(rep.pixelsHigh()).ok()?;
    let size = NSSize::new(as_f64(width), as_f64(height));
    let normal = blank(size, 1.0)?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&normal)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    rep.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), size));
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    let row = usize::try_from(normal.bytesPerRow()).ok()?;
    let data = normal.bitmapData();
    if data.is_null() {
        return None;
    }
    // SAFETY: a packed, meshed rep owns `bytesPerRow * pixelsHigh` bytes at `bitmapData`, and
    // `normal` outlives the slice, which is copied before it drops.
    let bytes = unsafe { std::slice::from_raw_parts(data, row * height) };
    let mut rgba = Vec::with_capacity(width * height * 4);
    for line in bytes.chunks_exact(row) {
        rgba.extend_from_slice(&line[..width * 4]);
    }
    Some(Pixels {
        width,
        height,
        rgba,
    })
}

fn as_f64(pixels: usize) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "an image's side in pixels is exact in a float"
    )]
    let value = pixels as f64;
    value
}

/// Writes `rep` as a PNG.
pub(super) fn write_png(rep: &NSBitmapImageRep, path: &Path) -> Result<(), String> {
    // SAFETY: an empty properties dictionary is valid for every file type.
    let data = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or_else(|| format!("{}: could not encode a PNG", path.display()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(path, data.to_vec()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads a PNG (or any image `AppKit` reads) as a bitmap.
pub(super) fn read_png(path: &Path) -> Result<Retained<NSBitmapImageRep>, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(&bytes))
        .ok_or_else(|| format!("{}: not an image", path.display()))
}

/// A bitmap of `pixels`, to write a diff image.
pub(super) fn rep_of(pixels: &Pixels) -> Option<Retained<NSBitmapImageRep>> {
    let rep = blank(
        NSSize::new(as_f64(pixels.width), as_f64(pixels.height)),
        1.0,
    )?;
    fill(&rep, pixels);
    Some(rep)
}

fn fill(rep: &NSBitmapImageRep, pixels: &Pixels) {
    let Ok(row) = usize::try_from(rep.bytesPerRow()) else {
        return;
    };
    let data = rep.bitmapData();
    if data.is_null() || row < pixels.width * 4 {
        return;
    }
    // SAFETY: the rep was made for exactly `pixels.width` x `pixels.height` packed RGBA
    // pixels and owns `bytesPerRow * height` bytes; nothing else borrows them meanwhile.
    let bytes = unsafe { std::slice::from_raw_parts_mut(data, row * pixels.height) };
    for (line, source) in bytes
        .chunks_exact_mut(row)
        .zip(pixels.rgba.chunks_exact(pixels.width * 4))
    {
        line[..source.len()].copy_from_slice(source);
    }
}

/// A made-up page picture for thumbnails, the same on every run: a coloured header band
/// (`hue` in 0…1), a title bar, and grey lines of text on the theme-neutral page white.
#[must_use]
pub fn page_picture(width: usize, height: usize, hue: f64) -> Option<Retained<NSImage>> {
    let header = hsv(hue, 0.55, 0.85);
    let mut rgba = vec![255_u8; width * height * 4];
    let mut paint = |x0: usize, y0: usize, x1: usize, y1: usize, color: [u8; 3]| {
        for y in y0..y1.min(height) {
            for x in x0..x1.min(width) {
                let at = (y * width + x) * 4;
                rgba[at..at + 3].copy_from_slice(&color);
            }
        }
    };
    let band = height / 4;
    paint(0, 0, width, band, header);
    paint(
        width / 12,
        band / 3,
        width / 2,
        band / 3 + band / 5,
        [255, 255, 255],
    );
    let mut y = band + height / 12;
    let mut line = 0;
    while y + 6 < height {
        let end = width - width / 12 - (line % 3) * width / 7;
        paint(width / 12, y, end, y + 6, [205, 208, 214]);
        y += 16;
        line += 1;
    }
    let pixels = Pixels {
        width,
        height,
        rgba,
    };
    let rep = rep_of(&pixels)?;
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(as_f64(width), as_f64(height)));
    image.addRepresentation(&rep);
    Some(image)
}

fn hsv(hue: f64, saturation: f64, value: f64) -> [u8; 3] {
    let turn = hue.rem_euclid(1.0) * 6.0;
    let fraction = turn - turn.floor();
    let low = value * (1.0 - saturation);
    let falling = value * (1.0 - saturation * fraction);
    let rising = value * (1.0 - saturation * (1.0 - fraction));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a hue turn in 0…6, floored to its sector"
    )]
    let sector = turn.floor() as u8;
    let channels = match sector {
        0 => [value, rising, low],
        1 => [falling, value, low],
        2 => [low, value, rising],
        3 => [low, falling, value],
        4 => [rising, low, value],
        _ => [value, low, falling],
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "channels in 0…1, scaled and rounded into a byte"
    )]
    channels.map(|channel| (channel * 255.0).round() as u8)
}
