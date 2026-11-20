// The bridge entry points Rust calls (quickie_bridge.h): argument conversion only; the work is in
// TabRegistry.

#include "quickie/bridge/quickie_bridge.h"

#include <optional>
#include <string_view>

#include "base/check.h"
#include "base/functional/bind.h"
#include "base/location.h"
#include "base/task/sequenced_task_runner.h"
#include "content/public/browser/browser_thread.h"
#include "quickie/bridge/tab_registry.h"
#include "quickie_crates/adapters/engine-chromium/src/ffi.rs.h"
#include "url/gurl.h"

namespace quickie {

namespace {

GURL ToGurl(rust::Str url) {
  return GURL(std::string_view(url.data(), url.size()));
}

}  // namespace

bool CreateTab(std::uint64_t tab, rust::Str url, bool throwaway, std::uint64_t jar) {
  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
  return TabRegistry::Get().CreateTab(
      tab, ToGurl(url), throwaway ? std::optional<std::uint64_t>(jar) : std::nullopt);
}

bool ShowTab(std::uint64_t window, HostView* host_view, std::uint64_t tab) {
  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
  return TabRegistry::Get().ShowTab(window, host_view, tab);
}

bool NavigateTab(std::uint64_t tab, NavigationKind kind, rust::Str url) {
  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
  return TabRegistry::Get().Navigate(tab, kind, ToGurl(url));
}

bool CloseTab(std::uint64_t tab) {
  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
  return TabRegistry::Get().CloseTab(tab);
}

bool AdoptTab(std::uint64_t page, std::uint64_t tab) {
  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
  return TabRegistry::Get().AdoptTab(page, tab);
}

bool DestroyJar(std::uint64_t jar) {
  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
  return TabRegistry::Get().DestroyJar(jar);
}

void ScheduleWake() {
  DCHECK_CURRENTLY_ON(content::BrowserThread::UI);
  // base/task/sequenced_task_runner.h. TODO(verify): RunWake, generated into ffi.rs.h, binds to
  // base::BindOnce as a plain function (cxx emits extern "Rust" functions as free functions).
  base::SequencedTaskRunner::GetCurrentDefault()->PostTask(FROM_HERE,
                                                           base::BindOnce(&RunWake));
}

}  // namespace quickie
