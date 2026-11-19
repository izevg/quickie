// The strong definition of the hook that the BrowserWindow factory override asks first
// (engine/overrides/chrome/browser/ui/views/frame/browser_window_factory.cc). It replaces the weak
// default there when //quickie is linked, which only the product does.

#include <memory>

#include "chrome/browser/ui/browser_window.h"
#include "chrome/browser/ui/browser_window_deleter.h"
#include "quickie/bridge/tab_registry.h"
#include "quickie/browser_window/quickie_browser_window.h"

namespace quickie {

std::unique_ptr<BrowserWindow, BrowserWindowDeleter> MaybeCreateBrowserWindow(
    BrowserWindowInterface* browser) {
  return std::unique_ptr<BrowserWindow, BrowserWindowDeleter>(
      TabRegistry::Get().MaybeCreateWindow(browser).release());
}

}  // namespace quickie
