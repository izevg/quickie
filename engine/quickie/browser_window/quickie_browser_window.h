// A BrowserWindow without Views' BrowserView (docs/architecture.md, section 4.4): the Browser's
// active WebContents view (WebContentsViewCocoa, holding RenderWidgetHostViewCocoa) is added as a
// subview of an NSView the Rust shell owns. No frame, toolbar, tab strip or bubbles of Chromium's:
// the shell draws all UI natively.
//
// The method list mirrors the pure virtuals of BrowserWindow and ui::BaseWindow at Chromium
// 156.0.8078.109 (chrome/browser/ui/browser_window.h, ui/base/base_window.h), cross-checked with
// the other BrowserWindow without BrowserView in that tree,
// chrome/browser/ui/webui_browser/webui_browser_window.h. Methods marked "Stub (P1)" do nothing
// useful yet; the implementation file says what each one should become.

#ifndef QUICKIE_BROWSER_WINDOW_QUICKIE_BROWSER_WINDOW_H_
#define QUICKIE_BROWSER_WINDOW_QUICKIE_BROWSER_WINDOW_H_

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "base/memory/raw_ptr.h"
#include "chrome/browser/ui/browser_window.h"

class BrowserWindowInterface;

namespace quickie {

class QuickieBrowserWindow : public BrowserWindow {
 public:
  // `window` is the shell window's id and `host_view` its NSView, or 0 and nullptr for a Browser
  // Chromium created on its own, which stays off screen until the shell shows one of its pages.
  QuickieBrowserWindow(BrowserWindowInterface* browser, std::uint64_t window, void* host_view);
  QuickieBrowserWindow(const QuickieBrowserWindow&) = delete;
  QuickieBrowserWindow& operator=(const QuickieBrowserWindow&) = delete;
  ~QuickieBrowserWindow() override;

  // The QuickieBrowserWindow of `browser`, or nullptr if it has another kind of window.
  // (Chromium builds without RTTI, so this is a lookup, not a cast.)
  static QuickieBrowserWindow* FromBrowser(BrowserWindowInterface* browser);

  std::uint64_t shell_window() const { return window_; }

  // Whether this Browser's active page fills the shell window's host view. A shell window has a
  // Browser per profile it shows; exactly one of them is attached.
  void SetAttached(bool attached);

  // ui::BaseWindow:
  bool IsActive() const override;
  bool IsMaximized() const override;
  bool IsMinimized() const override;
  bool IsFullscreen() const override;
  gfx::NativeWindow GetNativeWindow() const override;
  gfx::Rect GetRestoredBounds() const override;
  ui::mojom::WindowShowState GetRestoredState() const override;
  gfx::Rect GetBounds() const override;
  void Show() override;
  void Hide() override;
  bool IsVisible() const override;
  void ShowInactive() override;
  void Close() override;
  void Activate() override;
  void Deactivate() override;
  void Maximize() override;
  void Minimize() override;
  void Restore() override;
  void SetBounds(const gfx::Rect& bounds) override;
  void FlashFrame(bool flash) override;
  ui::ZOrderLevel GetZOrderLevel() const override;
  void SetZOrderLevel(ui::ZOrderLevel order) override;

  // BrowserWindow:
  bool IsOnCurrentWorkspace() const override;
  bool IsVisibleOnScreen() const override;
  void SetTopControlsShownRatio(content::WebContents* web_contents, float ratio) override;
  bool DoBrowserControlsShrinkRendererSize(
      const content::WebContents* contents) const override;
  ui::NativeTheme* GetNativeTheme() override;
  const ui::ThemeProvider* GetThemeProvider() const override;
  const ui::ColorProvider* GetColorProvider() const override;
  int GetTopControlsHeight() const override;
  void SetTopControlsGestureScrollInProgress(bool in_progress) override;
  std::vector<StatusBubble*> GetStatusBubbles() override;
  void UpdateTitleBar() override;
  void UpdateLoadingAnimations(bool is_visible) override;
  void OnActiveTabChanged(content::WebContents* old_contents,
                          content::WebContents* new_contents,
                          int index,
                          int reason) override;
  void OnTabDetached(content::WebContents* contents, bool was_active) override;
  gfx::Size GetContentsSize() const override;
  void SetContentsSize(const gfx::Size& size) override;
  autofill::AutofillBubbleHandler* GetAutofillBubbleHandler() override;
  LocationBar* GetLocationBar() const override;
  ui::AcceleratorProvider* GetAcceleratorProvider() override;
  PinnedToolbarActions* GetPinnedToolbarActions() override;
  void SetFocusToLocationBar(bool is_user_initiated) override;
  void UpdateReloadStopState(bool is_loading, bool force) override;
  void UpdateToolbar(content::WebContents* contents) override;
  bool UpdateToolbarSecurityState() override;
  void UpdateCustomTabBarVisibility(bool visible, bool animate) override;
  void ResetToolbarTabState(content::WebContents* contents) override;
  void FocusToolbar() override;
  void ToolbarSizeChanged(bool is_animating) override;
  void TabDraggingStatusChanged(bool is_dragging) override;
  void LinkOpeningFromGesture(WindowOpenDisposition disposition) override;
  void FocusAppMenu() override;
  bool IsTabStripEditable() const override;
  void DisableTabStripEditingForTesting() override;
  bool IsToolbarVisible() const override;
  bool IsToolbarShowing() const override;
  bool IsLocationBarVisible() const override;
  void ShowUpdateChromeDialog() override;
  void ShowIntentPickerBubble(std::vector<apps::IntentPickerAppInfo> app_info,
                              bool show_stay_in_chrome,
                              bool show_remember_selection,
                              apps::IntentPickerBubbleType bubble_type,
                              const std::optional<url::Origin>& initiating_origin,
                              IntentPickerResponse callback) override;
  void ShowBookmarkBubble(const GURL& url, bool already_bookmarked) override;
  ShowTranslateBubbleResult ShowTranslateBubble(content::WebContents* contents,
                                                translate::TranslateStep step,
                                                const std::string& source_language,
                                                const std::string& target_language,
                                                translate::TranslateErrors error_type,
                                                bool is_user_gesture) override;
  DownloadBubbleUIController* GetDownloadBubbleUIController() override;
  void ConfirmBrowserCloseWithPendingDownloads(
      int download_count,
      DownloadCloseType dialog_type,
      base::OnceCallback<void(bool)> callback) override;
  void ShowAppMenu() override;
  void PreHandleDragUpdate(const content::DropData& drop_data,
                           const gfx::PointF& point) override;
  void PreHandleDragExit() override;
  void HandleDragEnded() override;
  content::KeyboardEventProcessingResult PreHandleKeyboardEvent(
      const input::NativeWebKeyboardEvent& event) override;
  bool HandleKeyboardEvent(const input::NativeWebKeyboardEvent& event) override;
  std::unique_ptr<FindBar> CreateFindBar() override;
  web_modal::WebContentsModalDialogHost* GetWebContentsModalDialogHost() override;
  web_modal::WebContentsModalDialogHost* GetWebContentsModalDialogHostFor(
      content::WebContents* web_contents) override;
  void ShowAvatarBubbleFromAvatarButton(bool is_source_accelerator) override;
  void MaybeShowProfileSwitchIPH() override;
  void MaybeShowSupervisedUserProfileSignInIPH() override;
  void ShowHatsDialog(const std::string& site_id,
                      const std::optional<std::string>& hats_histogram_name,
                      const std::optional<uint64_t> hats_survey_ukm_id,
                      base::OnceClosure success_callback,
                      base::OnceClosure failure_callback,
                      const SurveyBitsData& product_specific_bits_data,
                      const SurveyStringData& product_specific_string_data) override;
  ExclusiveAccessContext* GetExclusiveAccessContext() override;
  std::string GetWorkspace() const override;
  bool IsVisibleOnAllWorkspaces() const override;
  void ShowEmojiPanel() override;
  std::unique_ptr<content::EyeDropper> OpenEyeDropper(
      content::RenderFrameHost* frame,
      content::EyeDropperListener* listener) override;
  void ShowCaretBrowsingDialog() override;
  void CreateTabSearchBubble() override;
  void CloseTabSearchBubble() override;
  void ShowIncognitoClearBrowsingDataDialog() override;
  void ShowIncognitoHistoryDisclaimerDialog() override;
  bool IsUnframedModeEnabled() const override;
  bool GetCanResize() override;
  ui::mojom::WindowShowState GetWindowShowState() const override;
  void ShowChromeLabs() override;
  BrowserView* AsBrowserView() override;

 protected:
  // BrowserWindow (called through BrowserWindowDeleter when the Browser lets go of it):
  void DeleteBrowserWindow() final;

 private:
  // The Objective-C state (the host NSView, strongly held), kept out of this C++ header.
  struct Host;

  // Puts the active page's view into the host view, or takes it out.
  void UpdateHostedContents();

  const raw_ptr<BrowserWindowInterface> browser_;
  const std::uint64_t window_;
  std::unique_ptr<Host> host_;
  bool attached_ = false;
  bool visible_ = false;
  bool closing_ = false;
  raw_ptr<content::WebContents> hosted_contents_ = nullptr;
};

}  // namespace quickie

#endif  // QUICKIE_BROWSER_WINDOW_QUICKIE_BROWSER_WINDOW_H_
