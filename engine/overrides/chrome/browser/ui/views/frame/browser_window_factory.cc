// Copyright 2014 The Chromium Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

// Quickie override (engine/overrides, copied over the tree by engine/scripts/apply).
// Why: Quickie hosts web contents in windows its Rust shell owns, without Views' BrowserView
// (docs/architecture.md, sections 3 and 4.4). This is the stock factory of Chromium
// 156.0.8078.109 plus one hook, quickie::MaybeCreateBrowserWindow, asked first. A whole-file
// override instead of a patch because the factory is the seam itself: on a rebase, diff this file
// against the new upstream one and carry the hook over (search for "Quickie").

#include <memory>

#include "build/build_config.h"
#include "chrome/browser/ui/browser_window_deleter.h"
#include "chrome/browser/ui/fullscreen/browser_window_fullscreen_controller.h"
#include "chrome/browser/ui/views/frame/browser_native_widget_factory.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/frame/browser_widget.h"
#include "chrome/browser/ui/webui_browser/webui_browser.h"
#include "chrome/browser/ui/webui_browser/webui_browser_window.h"
#include "chrome/browser/ui/window_feature_controller/window_feature_controller.h"
#include "ui/views/widget/widget.h"

#if defined(USE_AURA)
#include "ui/aura/client/aura_constants.h"
#include "ui/aura/window.h"
#include "ui/aura/window_occlusion_tracker.h"
#endif

#if BUILDFLAG(IS_CHROMEOS)
#include "chrome/browser/ui/views/frame/browser_view_ash.h"
#include "chrome/browser/ui/window_feature_controller/window_feature_controller.h"
#include "chromeos/components/kiosk/kiosk_utils.h"
#endif

// Quickie: returns the window for a browser Quickie hosts, or nullptr to keep Chromium's. The
// strong definition is in //quickie/browser_window/window_factory.mm; this weak default keeps
// targets that do not link //quickie (unit_tests, browser_tests) linking.
// TODO(verify): the strong definition wins at link time (see the same note in
// engine/patches/startup/browser-main-parts.patch).
namespace quickie {
__attribute__((weak)) std::unique_ptr<BrowserWindow, BrowserWindowDeleter>
MaybeCreateBrowserWindow(BrowserWindowInterface* browser) {
  return nullptr;
}
}  // namespace quickie

// static
std::unique_ptr<BrowserWindow, BrowserWindowDeleter>
BrowserWindow::CreateBrowserWindow(BrowserWindowInterface* browser,
                                   bool user_gesture,
                                   bool in_tab_dragging) {
  // Quickie: asked first, so its window wins over WebUIBrowserWindow and BrowserView.
  if (auto quickie_window = quickie::MaybeCreateBrowserWindow(browser)) {
    return quickie_window;
  }

  if (webui_browser::IsWebUIBrowserEnabled() &&
      browser->GetType() == BrowserWindowInterface::Type::TYPE_NORMAL) {
    return std::unique_ptr<BrowserWindow, BrowserWindowDeleter>(
        new WebUIBrowserWindow(browser));
  }

#if defined(USE_AURA)
  // Avoid generating too many occlusion tracking calculation events before this
  // function returns. The occlusion status will be computed only once once this
  // function returns.
  // See crbug.com/40171404#comment5
  aura::WindowOcclusionTracker::ScopedPause pause_occlusion;
#endif
  // Create the view and the frame. The frame will attach itself via the view
  // so we don't need to do anything with the pointer.
  BrowserView* view = nullptr;
#if BUILDFLAG(IS_CHROMEOS)
  view = new BrowserViewAsh(browser);
#else
  view = new BrowserView(browser);
#endif
  auto browser_widget = std::make_unique<BrowserWidget>(view);
  view->set_browser_widget(std::move(browser_widget));
  if (in_tab_dragging) {
    view->browser_widget()->SetTabDragKind(TabDragKind::kAllTabs);
  }
  view->browser_widget()->InitBrowserWidget();

#if BUILDFLAG(IS_MAC)
  if (WindowFeatureController::From(view->browser())
          ->UsesImmersiveFullscreenMode()) {
    // This needs to happen after BrowserWidget has been initialized. It creates
    // a new Widget that copies the theme from BrowserWidget.
    view->CreateMacOverlayView();
  }
#endif  // IS_MAC

#if defined(USE_AURA)
  // For now, all browser windows are true. This only works when USE_AURA
  // because it requires gfx::NativeWindow to be an aura::Window*.
  view->GetWidget()->GetNativeWindow()->SetProperty(
      aura::client::kCreatedByUserGesture, user_gesture);
#endif
#if BUILDFLAG(IS_CHROMEOS)
  if (chromeos::IsKioskSession()) {
    BrowserWindowFullscreenController::From(browser)->SetForceFullscreen(true);
  }
#endif

  return std::unique_ptr<BrowserWindow, BrowserWindowDeleter>(view);
}

// static
const BrowserWindow* BrowserWindow::FromBrowser(
    const BrowserWindowInterface* browser) {
  // See https://crbug.com/496674143 for the umbrella effort to remove
  // Browser::window() in favor of this helper.
  if (!browser) {
    return nullptr;
  }
  // The two production subclasses (BrowserView, WebUIBrowserWindow) each
  // register themselves under the BrowserWidget's NativeWindow properties, so
  // for any given Browser at most one of these lookups will succeed.
  if (auto* view = BrowserView::GetBrowserViewForBrowser(browser)) {
    return view;
  }
  if (auto* webui = WebUIBrowserWindow::FromBrowser(browser)) {
    return webui;
  }
  // Fallback for BrowserWindow implementations that are not reachable through
  // the NativeWindow property table (notably TestBrowserWindow in unit tests,
  // and during window teardown, when the NativeWindow lookups above no longer
  // resolve but callers may still need the window - e.g. to save workspace
  // state or update commands as tabs close). The window returned by
  // GetWindow() is always a BrowserWindow, so the downcast is safe.
  return static_cast<const BrowserWindow*>(browser->GetWindow());
}

// static
BrowserWindow* BrowserWindow::FromBrowser(BrowserWindowInterface* browser) {
  // Implement the non-const overload in terms of the const one and cast the
  // constness back off. Safe because the caller supplied a non-const browser.
  // See //styleguide/c++/const.md#classes-of-const-in_correctness.
  return const_cast<BrowserWindow*>(
      FromBrowser(static_cast<const BrowserWindowInterface*>(browser)));
}
