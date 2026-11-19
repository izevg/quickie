// See quickie_browser_window.h. "Stub (P1)" marks a BrowserWindow method that does nothing useful
// yet; each says what it should become. Several stubs return nullptr where Chromium may not expect
// it; those are TODO(verify) and listed in engine/README.md.

#include "quickie/browser_window/quickie_browser_window.h"

#import <AppKit/AppKit.h>

#include <map>
#include <utility>

#include "base/check.h"
#include "base/functional/callback.h"
#include "base/no_destructor.h"
#include "base/notimplemented.h"
#include "chrome/browser/themes/theme_service.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/find_bar/find_bar.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/unload_controller.h"
#include "components/input/native_web_keyboard_event.h"
#include "content/public/browser/eye_dropper.h"
#include "content/public/browser/keyboard_event_processing_result.h"
#include "content/public/browser/visibility.h"
#include "content/public/browser/web_contents.h"
#include "quickie/bridge/tab_registry.h"
#include "quickie/keyboard/key_equivalents.h"
#include "ui/base/mojom/window_show_state.mojom.h"
#include "ui/color/color_provider_manager.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/geometry/size.h"
#include "ui/gfx/mac/coordinate_conversion.h"
#include "ui/native_theme/native_theme.h"

namespace quickie {

namespace {

// Browser -> its QuickieBrowserWindow. Chromium builds without RTTI, so FromBrowser looks it up.
std::map<BrowserWindowInterface*, QuickieBrowserWindow*>& Windows() {
  static base::NoDestructor<std::map<BrowserWindowInterface*, QuickieBrowserWindow*>> windows;
  return *windows;
}

}  // namespace

struct QuickieBrowserWindow::Host {
  // Strong: the shell must not free the view while a page sits in it. ARC (Chromium builds .mm
  // files with -fobjc-arc).
  NSView* __strong view = nil;
};

QuickieBrowserWindow::QuickieBrowserWindow(BrowserWindowInterface* browser,
                                           std::uint64_t window,
                                           void* host_view)
    : browser_(browser), window_(window), host_(std::make_unique<Host>()) {
  host_->view = (__bridge NSView*)host_view;
  Windows()[browser] = this;
}

QuickieBrowserWindow::~QuickieBrowserWindow() {
  SetAttached(false);
  // Before erasing: the registry finds its entry through FromBrowser.
  TabRegistry::Get().OnWindowDestroyed(this);
  Windows().erase(browser_);
}

// static
QuickieBrowserWindow* QuickieBrowserWindow::FromBrowser(BrowserWindowInterface* browser) {
  auto it = Windows().find(browser);
  return it == Windows().end() ? nullptr : it->second;
}

void QuickieBrowserWindow::SetAttached(bool attached) {
  attached_ = attached;
  UpdateHostedContents();
}

void QuickieBrowserWindow::UpdateHostedContents() {
  content::WebContents* active = nullptr;
  if (attached_ && visible_ && host_->view && !closing_) {
    active = browser_->GetTabStripModel()->GetActiveWebContents();
  }
  if (active == hosted_contents_) {
    return;
  }
  if (hosted_contents_) {
    // content/public/browser/web_contents.h: GetNativeView() is the WebContentsViewCocoa
    // (ui/gfx/native_ui_types.h: gfx::NativeView::GetNativeNSView()).
    [hosted_contents_->GetNativeView().GetNativeNSView() removeFromSuperview];
    // Without a views::WebView nothing propagates visibility, so do it by hand, as
    // chrome/browser/ui/webui_browser/webui_browser_window.cc does in OnActiveTabChanged.
    hosted_contents_->UpdateWebContentsVisibility(content::Visibility::HIDDEN);
  }
  hosted_contents_ = active;
  if (!active) {
    return;
  }
  NSView* page = active->GetNativeView().GetNativeNSView();
  page.frame = host_->view.bounds;
  page.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
  // Below the shell's own subviews: overlays (palette, toasts) stay on top of the page.
  // TODO(verify): the shell draws overlays in sibling views or layers above this one; a page
  // view added later must not cover them.
  [host_->view addSubview:page positioned:NSWindowBelow relativeTo:nil];
  active->UpdateWebContentsVisibility(content::Visibility::VISIBLE);
}

// ui::BaseWindow ---------------------------------------------------------------------------------

bool QuickieBrowserWindow::IsActive() const {
  return attached_ && host_->view.window.isKeyWindow;
}

bool QuickieBrowserWindow::IsMaximized() const {
  return host_->view.window.zoomed;
}

bool QuickieBrowserWindow::IsMinimized() const {
  return host_->view.window.miniaturized;
}

bool QuickieBrowserWindow::IsFullscreen() const {
  return (host_->view.window.styleMask & NSWindowStyleMaskFullScreen) != 0;
}

gfx::NativeWindow QuickieBrowserWindow::GetNativeWindow() const {
  NSWindow* window = host_->view.window;
  return window ? gfx::NativeWindow(window) : gfx::NativeWindow();
}

gfx::Rect QuickieBrowserWindow::GetRestoredBounds() const {
  return GetBounds();
}

ui::mojom::WindowShowState QuickieBrowserWindow::GetRestoredState() const {
  return ui::mojom::WindowShowState::kDefault;
}

gfx::Rect QuickieBrowserWindow::GetBounds() const {
  NSWindow* window = host_->view.window;
  return window ? gfx::ScreenRectFromNSRect(window.frame) : gfx::Rect();
}

void QuickieBrowserWindow::Show() {
  // The shell owns the NSWindow and shows it; for Chromium, "shown" means the page may draw.
  visible_ = true;
  UpdateHostedContents();
}

void QuickieBrowserWindow::Hide() {
  visible_ = false;
  UpdateHostedContents();
}

bool QuickieBrowserWindow::IsVisible() const {
  return visible_ && attached_;
}

void QuickieBrowserWindow::ShowInactive() {
  Show();
}

void QuickieBrowserWindow::Close() {
  // Mirrors chrome/browser/ui/webui_browser/webui_browser_window.cc, OnWindowCloseRequested:
  // beforeunload handlers may cancel (then Chromium calls Close() again later); otherwise the
  // tabs close, and once the strip is empty Chromium deletes the Browser, which deletes this
  // window through BrowserWindowDeleter.
  // TODO(verify): that the Browser really is deleted without a views::Widget closing; compare
  // BrowserView's close path at the pinned version.
  UnloadController* unload = UnloadController::From(browser_);
  if (!unload->HandleBeforeClose()) {
    return;
  }
  unload->OnWindowClosing();
  closing_ = true;
  UpdateHostedContents();
}

void QuickieBrowserWindow::Activate() {
  // Stub (P1): the shell decides which window is key; it should be asked through the bridge.
  NOTIMPLEMENTED_LOG_ONCE();
}

void QuickieBrowserWindow::Deactivate() {
  // Stub (P1): as Activate.
  NOTIMPLEMENTED_LOG_ONCE();
}

void QuickieBrowserWindow::Maximize() {
  [host_->view.window zoom:nil];
}

void QuickieBrowserWindow::Minimize() {
  [host_->view.window miniaturize:nil];
}

void QuickieBrowserWindow::Restore() {
  [host_->view.window deminiaturize:nil];
}

void QuickieBrowserWindow::SetBounds(const gfx::Rect& bounds) {
  // Stub (P1): pages may resize their window (window.resizeTo, popups); the shell decides.
  NOTIMPLEMENTED_LOG_ONCE();
}

void QuickieBrowserWindow::FlashFrame(bool flash) {
  // Stub (P1): could bounce the Dock icon ([NSApp requestUserAttention:]).
  NOTIMPLEMENTED_LOG_ONCE();
}

ui::ZOrderLevel QuickieBrowserWindow::GetZOrderLevel() const {
  return ui::ZOrderLevel::kNormal;
}

void QuickieBrowserWindow::SetZOrderLevel(ui::ZOrderLevel order) {
  // Stub (P1): only picture in picture asks, and it keeps Chromium's window.
  NOTIMPLEMENTED_LOG_ONCE();
}

// BrowserWindow ----------------------------------------------------------------------------------

bool QuickieBrowserWindow::IsOnCurrentWorkspace() const {
  return host_->view.window.onActiveSpace;
}

bool QuickieBrowserWindow::IsVisibleOnScreen() const {
  return IsVisible() && host_->view.window.visible;
}

void QuickieBrowserWindow::SetTopControlsShownRatio(content::WebContents* web_contents,
                                                    float ratio) {
  // No top controls: content first.
}

bool QuickieBrowserWindow::DoBrowserControlsShrinkRendererSize(
    const content::WebContents* contents) const {
  return false;
}

ui::NativeTheme* QuickieBrowserWindow::GetNativeTheme() {
  return ui::NativeTheme::GetInstanceForNativeUi();
}

const ui::ThemeProvider* QuickieBrowserWindow::GetThemeProvider() const {
  // chrome/browser/themes/theme_service.h, as WebUIBrowserWindow does for non-app browsers.
  return &ThemeService::GetThemeProviderForProfile(browser_->GetProfile());
}

const ui::ColorProvider* QuickieBrowserWindow::GetColorProvider() const {
  // ui/native_theme/native_theme.h: a null custom theme is the default theme, which is what
  // Quickie wants (WebUIBrowserWindow passes its profile's theme supplier instead).
  return ui::ColorProviderManager::Get().GetColorProviderFor(
      ui::NativeTheme::GetInstanceForNativeUi()->GetColorProviderKey(nullptr));
}

int QuickieBrowserWindow::GetTopControlsHeight() const {
  return 0;
}

void QuickieBrowserWindow::SetTopControlsGestureScrollInProgress(bool in_progress) {}

std::vector<StatusBubble*> QuickieBrowserWindow::GetStatusBubbles() {
  // No status bubble: link targets show in the shell's OriginChip, on request.
  return {};
}

void QuickieBrowserWindow::UpdateTitleBar() {
  // Titles reach the shell as EngineEventKind::TitleChanged.
}

void QuickieBrowserWindow::UpdateLoadingAnimations(bool is_visible) {
  // Stub (P1): the shell's LoadProgressLine should learn about loading through an event.
}

void QuickieBrowserWindow::OnActiveTabChanged(content::WebContents* old_contents,
                                              content::WebContents* new_contents,
                                              int index,
                                              int reason) {
  UpdateHostedContents();
}

void QuickieBrowserWindow::OnTabDetached(content::WebContents* contents, bool was_active) {
  if (contents == hosted_contents_) {
    [hosted_contents_->GetNativeView().GetNativeNSView() removeFromSuperview];
    hosted_contents_ = nullptr;
  }
}

gfx::Size QuickieBrowserWindow::GetContentsSize() const {
  NSView* view = host_->view;
  return view ? gfx::Size(view.bounds.size.width, view.bounds.size.height) : gfx::Size();
}

void QuickieBrowserWindow::SetContentsSize(const gfx::Size& size) {
  // Stub (P1): as SetBounds.
  NOTIMPLEMENTED_LOG_ONCE();
}

autofill::AutofillBubbleHandler* QuickieBrowserWindow::GetAutofillBubbleHandler() {
  // Stub (P1): TODO(verify) callers handle nullptr; autofill bubbles become native sheets (P3).
  NOTIMPLEMENTED_LOG_ONCE();
  return nullptr;
}

LocationBar* QuickieBrowserWindow::GetLocationBar() const {
  // Stub (P1): there is no location bar; the shell's AddressOverlay replaces it.
  // TODO(verify): browser commands that reach for the location bar (chrome::FocusLocationBar,
  // omnibox paths) must not run; WebUIBrowserWindow returns a stub LocationBar instead.
  return nullptr;
}

ui::AcceleratorProvider* QuickieBrowserWindow::GetAcceleratorProvider() {
  // Shortcuts live in the Rust keymap, not in Chromium's accelerator table.
  return nullptr;
}

PinnedToolbarActions* QuickieBrowserWindow::GetPinnedToolbarActions() {
  return nullptr;
}

void QuickieBrowserWindow::SetFocusToLocationBar(bool is_user_initiated) {
  // Stub (P1): should open the shell's AddressOverlay through an event.
  NOTIMPLEMENTED_LOG_ONCE();
}

void QuickieBrowserWindow::UpdateReloadStopState(bool is_loading, bool force) {}

void QuickieBrowserWindow::UpdateToolbar(content::WebContents* contents) {}

bool QuickieBrowserWindow::UpdateToolbarSecurityState() {
  return false;
}

void QuickieBrowserWindow::UpdateCustomTabBarVisibility(bool visible, bool animate) {}

void QuickieBrowserWindow::ResetToolbarTabState(content::WebContents* contents) {}

void QuickieBrowserWindow::FocusToolbar() {}

void QuickieBrowserWindow::ToolbarSizeChanged(bool is_animating) {}

void QuickieBrowserWindow::TabDraggingStatusChanged(bool is_dragging) {}

void QuickieBrowserWindow::LinkOpeningFromGesture(WindowOpenDisposition disposition) {}

void QuickieBrowserWindow::FocusAppMenu() {}

bool QuickieBrowserWindow::IsTabStripEditable() const {
  return true;
}

void QuickieBrowserWindow::DisableTabStripEditingForTesting() {}

bool QuickieBrowserWindow::IsToolbarVisible() const {
  return false;
}

bool QuickieBrowserWindow::IsToolbarShowing() const {
  return false;
}

bool QuickieBrowserWindow::IsLocationBarVisible() const {
  return false;
}

void QuickieBrowserWindow::ShowUpdateChromeDialog() {
  // Updates are Sparkle's (architecture section 1), never Chromium's.
}

void QuickieBrowserWindow::ShowIntentPickerBubble(
    std::vector<apps::IntentPickerAppInfo> app_info,
    bool show_stay_in_chrome,
    bool show_remember_selection,
    apps::IntentPickerBubbleType bubble_type,
    const std::optional<url::Origin>& initiating_origin,
    IntentPickerResponse callback) {
  // Stub (P1): no intent picker; the callback is dropped, which callers treat as dismissed.
  // TODO(verify): chrome/browser/apps/link_capturing callers tolerate a dropped callback.
  NOTIMPLEMENTED_LOG_ONCE();
}

void QuickieBrowserWindow::ShowBookmarkBubble(const GURL& url, bool already_bookmarked) {
  // Stub (P1): bookmarks are the shell's library, not Chromium's.
  NOTIMPLEMENTED_LOG_ONCE();
}

ShowTranslateBubbleResult QuickieBrowserWindow::ShowTranslateBubble(
    content::WebContents* contents,
    translate::TranslateStep step,
    const std::string& source_language,
    const std::string& target_language,
    translate::TranslateErrors error_type,
    bool is_user_gesture) {
  // Translate is off (docs/privacy.md).
  return ShowTranslateBubbleResult::kBrowserWindowNotValid;
}

DownloadBubbleUIController* QuickieBrowserWindow::GetDownloadBubbleUIController() {
  // Stub (P1): downloads become the shell's DownloadsPopover.
  return nullptr;
}

void QuickieBrowserWindow::ConfirmBrowserCloseWithPendingDownloads(
    int download_count,
    DownloadCloseType dialog_type,
    base::OnceCallback<void(bool)> callback) {
  // Stub (P1): never cancel downloads silently; keep the Browser open until the shell can ask.
  std::move(callback).Run(false);
}

void QuickieBrowserWindow::ShowAppMenu() {}

void QuickieBrowserWindow::PreHandleDragUpdate(const content::DropData& drop_data,
                                               const gfx::PointF& point) {}

void QuickieBrowserWindow::PreHandleDragExit() {}

void QuickieBrowserWindow::HandleDragEnded() {}

content::KeyboardEventProcessingResult QuickieBrowserWindow::PreHandleKeyboardEvent(
    const input::NativeWebKeyboardEvent& event) {
  return keyboard::OfferBeforePage(event)
             ? content::KeyboardEventProcessingResult::HANDLED
             : content::KeyboardEventProcessingResult::NOT_HANDLED;
}

bool QuickieBrowserWindow::HandleKeyboardEvent(const input::NativeWebKeyboardEvent& event) {
  // TODO(verify): BrowserView on macOS re-dispatches unhandled key equivalents to the main menu
  // here (chrome/browser/ui/views/frame/browser_view.cc, HandleKeyboardEvent). Quickie's menu
  // comes from the Rust shell, which gets them through OfferAfterPage instead.
  return keyboard::OfferAfterPage(event);
}

std::unique_ptr<FindBar> QuickieBrowserWindow::CreateFindBar() {
  // Stub (P1): find in page is the shell's FindBar over the engine's find API.
  // TODO(verify): FindBarController callers handle a null FindBar; nothing should create one
  // while the shell owns ⌘F.
  NOTIMPLEMENTED_LOG_ONCE();
  return nullptr;
}

web_modal::WebContentsModalDialogHost* QuickieBrowserWindow::GetWebContentsModalDialogHost() {
  // Stub (P1). TODO(verify): tab-modal dialogs (JavaScript alert/confirm, HTTP auth) look up this
  // host; with nullptr they may not show or may crash. WebUIBrowserWindow has a
  // WebUIBrowserModalDialogHost; the native E3 sheets replace this in P3. Riskiest stub here.
  NOTIMPLEMENTED_LOG_ONCE();
  return nullptr;
}

web_modal::WebContentsModalDialogHost* QuickieBrowserWindow::GetWebContentsModalDialogHostFor(
    content::WebContents* web_contents) {
  return GetWebContentsModalDialogHost();
}

void QuickieBrowserWindow::ShowAvatarBubbleFromAvatarButton(bool is_source_accelerator) {}

void QuickieBrowserWindow::MaybeShowProfileSwitchIPH() {}

void QuickieBrowserWindow::MaybeShowSupervisedUserProfileSignInIPH() {}

void QuickieBrowserWindow::ShowHatsDialog(
    const std::string& site_id,
    const std::optional<std::string>& hats_histogram_name,
    const std::optional<uint64_t> hats_survey_ukm_id,
    base::OnceClosure success_callback,
    base::OnceClosure failure_callback,
    const SurveyBitsData& product_specific_bits_data,
    const SurveyStringData& product_specific_string_data) {
  // Surveys never show (trackerless).
  std::move(failure_callback).Run();
}

ExclusiveAccessContext* QuickieBrowserWindow::GetExclusiveAccessContext() {
  // Stub (P1). TODO(verify): fullscreen and pointer lock go through this context; with nullptr,
  // requestFullscreen() and the fullscreen notice (architecture section 8) do not work and may
  // crash. WebUIBrowserWindow has WebUIBrowserExclusiveAccessContext.
  NOTIMPLEMENTED_LOG_ONCE();
  return nullptr;
}

std::string QuickieBrowserWindow::GetWorkspace() const {
  return std::string();
}

bool QuickieBrowserWindow::IsVisibleOnAllWorkspaces() const {
  return false;
}

void QuickieBrowserWindow::ShowEmojiPanel() {
  [NSApp orderFrontCharacterPalette:nil];
}

std::unique_ptr<content::EyeDropper> QuickieBrowserWindow::OpenEyeDropper(
    content::RenderFrameHost* frame,
    content::EyeDropperListener* listener) {
  // Stub (P1): <input type=color> eye dropper.
  NOTIMPLEMENTED_LOG_ONCE();
  return nullptr;
}

void QuickieBrowserWindow::ShowCaretBrowsingDialog() {}

void QuickieBrowserWindow::CreateTabSearchBubble() {}

void QuickieBrowserWindow::CloseTabSearchBubble() {}

void QuickieBrowserWindow::ShowIncognitoClearBrowsingDataDialog() {}

void QuickieBrowserWindow::ShowIncognitoHistoryDisclaimerDialog() {}

bool QuickieBrowserWindow::IsUnframedModeEnabled() const {
  return false;
}

bool QuickieBrowserWindow::GetCanResize() {
  return true;
}

ui::mojom::WindowShowState QuickieBrowserWindow::GetWindowShowState() const {
  if (IsFullscreen()) {
    return ui::mojom::WindowShowState::kFullscreen;
  }
  if (IsMinimized()) {
    return ui::mojom::WindowShowState::kMinimized;
  }
  if (IsMaximized()) {
    return ui::mojom::WindowShowState::kMaximized;
  }
  return ui::mojom::WindowShowState::kDefault;
}

void QuickieBrowserWindow::ShowChromeLabs() {}

BrowserView* QuickieBrowserWindow::AsBrowserView() {
  return nullptr;
}

void QuickieBrowserWindow::DeleteBrowserWindow() {
  delete this;
}

}  // namespace quickie
