//! Floating surfaces by elevation (`docs/design/system.md`, Elevation): Liquid Glass on macOS 26,
//! `NSVisualEffectView` before it, with the shadow, the hairline ring, and the content scrim.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSGlassEffectContainerView, NSGlassEffectView,
    NSGlassEffectViewStyle, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView,
};
use objc2_core_foundation::CGFloat;
use objc2_core_graphics::{CGMutablePath, CGPath};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CAShapeLayer, kCACornerCurveContinuous, kCAFillRuleEvenOdd};

use crate::tokens::{self, radius};

/// How far a surface floats above the page. E0, the page itself, has no surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    /// Indicators: clear glass, no shadow, no dimming.
    E1,
    /// Overlays: regular glass in a glass container, medium shadow, page dimmed 10 %.
    E2,
    /// Sheets: tinted glass, deep shadow, page dimmed 30 %.
    E3,
}

/// A CSS-style box shadow (`0 offset_y blur spread`) plus a 1 px white ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset_y: CGFloat,
    pub blur: CGFloat,
    pub spread: CGFloat,
    pub opacity: f32,
    pub ring_opacity: CGFloat,
}

impl Elevation {
    /// Black overlay over the page while a surface of this level is up.
    #[must_use]
    pub fn dimming(self) -> CGFloat {
        match self {
            Self::E1 => 0.0,
            Self::E2 => 0.10,
            Self::E3 => 0.30,
        }
    }

    #[must_use]
    pub fn shadow(self) -> Option<Shadow> {
        match self {
            Self::E1 => None,
            Self::E2 => Some(Shadow {
                offset_y: 20.0,
                blur: 48.0,
                spread: -10.0,
                opacity: 0.22,
                ring_opacity: 0.20,
            }),
            Self::E3 => Some(Shadow {
                offset_y: 32.0,
                blur: 72.0,
                spread: -12.0,
                opacity: 0.45,
                ring_opacity: 0.25,
            }),
        }
    }

    /// Corner radius for a surface of `height`: E1 is a pill, E2 and E3 are overlays.
    #[must_use]
    pub fn corner_radius(self, height: CGFloat) -> CGFloat {
        match self {
            Self::E1 => height / 2.0,
            Self::E2 | Self::E3 => radius::OVERLAY,
        }
    }

    fn glass_style(self) -> NSGlassEffectViewStyle {
        match self {
            Self::E1 => NSGlassEffectViewStyle::Clear,
            Self::E2 | Self::E3 => NSGlassEffectViewStyle::Regular,
        }
    }

    fn fallback_material(self) -> NSVisualEffectMaterial {
        match self {
            Self::E1 | Self::E2 => NSVisualEffectMaterial::Popover,
            Self::E3 => NSVisualEffectMaterial::HUDWindow,
        }
    }
}

/// Room around the surface for the shadow to fall into.
fn shadow_margin(shadow: Shadow) -> CGFloat {
    shadow.blur * 2.0 + shadow.offset_y.abs()
}

/// True when Liquid Glass exists (macOS 26 and later).
#[must_use]
pub fn liquid_glass_available() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// One floating surface. Add [`GlassSurface::view`] above the page and put content into
/// [`GlassSurface::content_view`]; never put another glass surface inside it.
pub struct GlassSurface {
    elevation: Elevation,
    root: Retained<NSView>,
    shadow: Option<Retained<NSView>>,
    material: Material,
    content: Retained<NSView>,
}

enum Material {
    Glass(Retained<NSGlassEffectView>),
    Fallback(Retained<NSVisualEffectView>),
    /// A story render's opaque stand-in ([`stand_in`]).
    #[cfg(feature = "stories")]
    StandIn(Retained<NSView>),
}

impl GlassSurface {
    #[must_use]
    pub fn new(elevation: Elevation, mtm: MainThreadMarker) -> Self {
        let root = NSView::new(mtm);
        let shadow = elevation.shadow().map(|_| {
            let view = NSView::new(mtm);
            view.setWantsLayer(true);
            root.addSubview(&view);
            view
        });
        let content = NSView::new(mtm);
        content.setAutoresizingMask(fill());
        content.setWantsLayer(true);
        let material = material(elevation, &root, &content, mtm);
        let surface = Self {
            elevation,
            root,
            shadow,
            material,
            content,
        };
        if elevation == Elevation::E3 {
            surface.set_tint(Some(&tokens::color::accent().colorWithAlphaComponent(0.12)));
        }
        surface
    }

    /// The view to add above the page.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.root
    }

    /// Where the surface's content goes; it fills the surface.
    #[must_use]
    pub fn content_view(&self) -> &NSView {
        &self.content
    }

    #[must_use]
    pub fn elevation(&self) -> Elevation {
        self.elevation
    }

    /// Tints the glass (E3 sheets). The fallback material has no tint.
    pub fn set_tint(&self, color: Option<&NSColor>) {
        if let Material::Glass(glass) = &self.material {
            glass.setTintColor(color);
        }
    }

    /// Places a surface of `size` at the top right of `host`, `space-lg` from the top and
    /// `space-xl` from the right edge, where popovers sit; it stays there as the window
    /// resizes. A host narrower than the surface gets it narrowed.
    pub fn place_top_right(&self, host: &NSView, size: NSSize) {
        let bounds = host.bounds();
        let width = size.width.min(bounds.size.width - tokens::space::XL * 2.0);
        self.set_frame(NSRect::new(
            NSPoint::new(
                (bounds.size.width - tokens::space::XL - width).round(),
                (bounds.size.height - tokens::space::LG - size.height).round(),
            ),
            NSSize::new(width, size.height),
        ));
        self.root.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
    }

    /// Places the surface in its superview's coordinates and lays out shape and shadow.
    pub fn set_frame(&self, frame: NSRect) {
        self.set_frame_rounded(frame, self.elevation.corner_radius(frame.size.height));
    }

    /// As [`Self::set_frame`], with corners of radius `corner` instead of the elevation's: a
    /// tall E1 panel is a rounded rectangle, not a pill.
    pub fn set_frame_rounded(&self, frame: NSRect, corner: CGFloat) {
        self.root.setFrame(frame);
        let size = frame.size;
        match &self.material {
            Material::Glass(glass) => glass.setCornerRadius(corner),
            Material::Fallback(view) => round(view, corner),
            #[cfg(feature = "stories")]
            Material::StandIn(view) => round(view, corner),
        }
        if let (Some(view), Some(shadow)) = (&self.shadow, self.elevation.shadow()) {
            layout_shadow(view, shadow, size, corner);
            ring(&self.content, shadow, corner);
        }
    }
}

fn fill() -> NSAutoresizingMaskOptions {
    NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable
}

fn glass(
    elevation: Elevation,
    root: &NSView,
    content: &NSView,
    mtm: MainThreadMarker,
) -> Retained<NSGlassEffectView> {
    let glass = NSGlassEffectView::new(mtm);
    glass.setStyle(elevation.glass_style());
    glass.setContentView(Some(content));
    glass.setAutoresizingMask(fill());
    if elevation == Elevation::E2 {
        let container = NSGlassEffectContainerView::new(mtm);
        container.setAutoresizingMask(fill());
        container.setContentView(Some(&glass));
        root.addSubview(&container);
        fit(&container, root);
    } else {
        root.addSubview(&glass);
    }
    fit(&glass, root);
    glass
}

/// Liquid Glass when the system has it, the visual effect view before; in a story render,
/// the stand-in.
fn material(
    elevation: Elevation,
    root: &NSView,
    content: &NSView,
    mtm: MainThreadMarker,
) -> Material {
    #[cfg(feature = "stories")]
    if crate::stories::glass_stand_in() {
        return Material::StandIn(stand_in(elevation, root, content, mtm));
    }
    if liquid_glass_available() {
        Material::Glass(glass(elevation, root, content, mtm))
    } else {
        Material::Fallback(fallback(elevation, root, content, mtm))
    }
}

/// What a story renders in place of glass: an opaque fill in the window background colour,
/// tinted with the accent for E3 as the glass is. Neither Liquid Glass nor the visual effect
/// view draws offscreen, and the stand-in makes renders independent of what is behind them.
#[cfg(feature = "stories")]
fn stand_in(
    elevation: Elevation,
    root: &NSView,
    content: &NSView,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let view = NSView::new(mtm);
    view.setWantsLayer(true);
    view.setAutoresizingMask(fill());
    let base = NSColor::windowBackgroundColor();
    let fill_color = if elevation == Elevation::E3 {
        base.blendedColorWithFraction_ofColor(0.12, &tokens::color::accent())
            .unwrap_or(base)
    } else {
        base
    };
    if let Some(layer) = view.layer() {
        layer.setBackgroundColor(Some(&fill_color.CGColor()));
    }
    view.addSubview(content);
    root.addSubview(&view);
    fit(&view, root);
    fit(content, &view);
    view
}

fn fallback(
    elevation: Elevation,
    root: &NSView,
    content: &NSView,
    mtm: MainThreadMarker,
) -> Retained<NSVisualEffectView> {
    let view = NSVisualEffectView::new(mtm);
    view.setMaterial(elevation.fallback_material());
    view.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
    view.setState(NSVisualEffectState::Active);
    view.setAutoresizingMask(fill());
    view.setWantsLayer(true);
    view.addSubview(content);
    root.addSubview(&view);
    fit(&view, root);
    fit(content, &view);
    view
}

/// Gives `view` the bounds of `host`; autoresizing keeps it there.
fn fit(view: &NSView, host: &NSView) {
    view.setFrame(host.bounds());
}

fn round(view: &NSView, corner: CGFloat) {
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(corner);
        // SAFETY: `kCACornerCurveContinuous` is an immutable string exported by Core Animation.
        layer.setCornerCurve(unsafe { kCACornerCurveContinuous });
        layer.setMasksToBounds(true);
    }
}

/// The 1 px white ring along the surface's edge, drawn by the content layer.
fn ring(content: &NSView, shadow: Shadow, corner: CGFloat) {
    if let Some(layer) = content.layer() {
        layer.setBorderWidth(1.0);
        layer.setBorderColor(Some(&tokens::white(shadow.ring_opacity).CGColor()));
        layer.setCornerRadius(corner);
        // SAFETY: `kCACornerCurveContinuous` is an immutable string exported by Core Animation.
        layer.setCornerCurve(unsafe { kCACornerCurveContinuous });
    }
}

/// Lays the shadow view around the surface. Like a CSS box shadow, the shadow is cut out under
/// the surface, so translucent glass never shows it from inside.
fn layout_shadow(view: &NSView, shadow: Shadow, size: NSSize, corner: CGFloat) {
    let margin = shadow_margin(shadow);
    view.setFrame(NSRect::new(
        NSPoint::new(-margin, -margin),
        NSSize::new(size.width + margin * 2.0, size.height + margin * 2.0),
    ));
    let Some(layer) = view.layer() else { return };
    let surface = NSRect::new(NSPoint::new(margin, margin), size);
    let inset = -shadow.spread;
    let casting = NSRect::new(
        NSPoint::new(margin + inset, margin + inset),
        NSSize::new(
            (size.width - inset * 2.0).max(0.0),
            (size.height - inset * 2.0).max(0.0),
        ),
    );
    layer.setShadowPath(Some(&rounded(casting, (corner - inset).max(0.0))));
    layer.setShadowColor(Some(&tokens::black(1.0).CGColor()));
    layer.setShadowOpacity(shadow.opacity);
    layer.setShadowRadius(shadow.blur / 2.0);
    // Layers of unflipped views are y-up: a shadow below the surface has a negative offset.
    layer.setShadowOffset(NSSize::new(0.0, -shadow.offset_y));

    let cutout = CGMutablePath::new();
    // SAFETY: a null transform means identity; the path is a live mutable path.
    unsafe { CGMutablePath::add_rect(Some(&cutout), std::ptr::null(), view.bounds()) };
    // SAFETY: as above.
    unsafe {
        CGMutablePath::add_rounded_rect(Some(&cutout), std::ptr::null(), surface, corner, corner);
    };
    let mask = CAShapeLayer::new();
    mask.setFrame(view.bounds());
    mask.setPath(Some(&cutout));
    // SAFETY: `kCAFillRuleEvenOdd` is an immutable string exported by Core Animation.
    mask.setFillRule(unsafe { kCAFillRuleEvenOdd });
    // SAFETY: the mask layer is new and owned by no other layer tree.
    unsafe { layer.setMask(Some(&mask)) };
}

fn rounded(rect: NSRect, corner: CGFloat) -> objc2_core_foundation::CFRetained<CGPath> {
    // SAFETY: a null transform means identity; the radius fits the rect (clamped by callers).
    unsafe { CGPath::with_rounded_rect(rect, corner, corner, std::ptr::null()) }
}

/// The scrim that dims the page under an E2 or E3 surface. It also swallows clicks on the page.
pub struct Scrim {
    view: Retained<NSView>,
}

impl Scrim {
    /// A scrim for `elevation`; E1 dims nothing and gets none.
    #[must_use]
    pub fn new(elevation: Elevation, mtm: MainThreadMarker) -> Option<Self> {
        let dimming = elevation.dimming();
        if dimming <= 0.0 {
            return None;
        }
        let view = NSView::new(mtm);
        view.setWantsLayer(true);
        view.setAutoresizingMask(fill());
        if let Some(layer) = view.layer() {
            layer.setBackgroundColor(Some(&tokens::black(dimming).CGColor()));
        }
        Some(Self { view })
    }

    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }
}

#[cfg(test)]
mod tests {
    use super::{Elevation, shadow_margin};

    #[test]
    fn dimming_grows_with_elevation() {
        assert!(Elevation::E1.dimming().abs() < f64::EPSILON);
        assert!((Elevation::E2.dimming() - 0.10).abs() < f64::EPSILON);
        assert!((Elevation::E3.dimming() - 0.30).abs() < f64::EPSILON);
    }

    #[test]
    fn indicators_cast_no_shadow() {
        assert_eq!(Elevation::E1.shadow(), None);
        let e2 = Elevation::E2.shadow();
        let e3 = Elevation::E3.shadow();
        assert!(e2.is_some_and(|s| (s.opacity - 0.22).abs() < f32::EPSILON));
        assert!(e3.is_some_and(|s| (s.opacity - 0.45).abs() < f32::EPSILON));
    }

    #[test]
    fn indicators_are_pills_and_overlays_use_the_overlay_radius() {
        assert!((Elevation::E1.corner_radius(30.0) - 15.0).abs() < f64::EPSILON);
        assert!((Elevation::E2.corner_radius(30.0) - 22.0).abs() < f64::EPSILON);
        assert!((Elevation::E3.corner_radius(300.0) - 22.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_shadow_margin_holds_the_whole_blur_and_offset() {
        let margin = Elevation::E2.shadow().map(shadow_margin);
        assert!(margin.is_some_and(|m| (m - 116.0).abs() < f64::EPSILON));
    }
}
