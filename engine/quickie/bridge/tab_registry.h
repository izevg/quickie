// The engine side of the domain's tabs: which WebContents is which TabId, which Browser hosts it
// in which shell window, and which profile is which jar.
//
// Model (docs/architecture.md, section 4.3): Chromium's TabStripModel is the truth about live
// pages, so every shown page lives in a Browser. A Browser is per (shell window, profile): one for
// the main profile and one per throwaway jar a window shows. Each Browser gets a
// QuickieBrowserWindow (//quickie/browser_window) through the BrowserWindow factory override; the
// shell window's NSView hosts the active page of whichever of its Browsers showed a tab last.
// A page created before it is first shown is a detached WebContents owned here (tabs are created
// lazily, but a restored tab is created only when shown, so this is short-lived).
//
// Throwaway jars: each is its own off-the-record profile of the main profile, so cookies,
// storage, cache and HSTS are in memory and die with the profile (ProfileDestroyer). The OTR id
// is made with Profile::OTRProfileID::CreateUniqueForDevTools(), because a non-primary OTR
// profile may host Browser windows only if it is a DevTools (or Media Router) one
// (chrome/browser/profiles/profile.cc, OTRProfileID::AllowsBrowserWindows, at 156.0.8078.109);
// this is the mechanism behind CDP's Target.createBrowserContext
// (chrome/browser/devtools/devtools_browser_context_manager.cc).
// TODO(verify): side effects of the DevTools prefix beyond AllowsBrowserWindows (search
// OTRProfileID::IsDevTools callers) and whether extensions stay off in those profiles, as they
// should for a throwaway jar. Fallback if it misbehaves: keep throwaway pages out of Browsers,
// as detached WebContents with our own WebContentsDelegate.

#ifndef QUICKIE_BRIDGE_TAB_REGISTRY_H_
#define QUICKIE_BRIDGE_TAB_REGISTRY_H_

#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <utility>
#include <vector>

#include "base/memory/raw_ptr.h"
#include "base/memory/raw_ptr_exclusion.h"
#include "base/no_destructor.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"

class BrowserWindowInterface;
class GURL;
class Profile;

namespace content {
class WebContents;
}

namespace quickie {

enum class NavigationKind : std::uint8_t;
class QuickieBrowserWindow;

class TabRegistry : public TabStripModelObserver {
 public:
  // The process-wide registry. UI thread only.
  static TabRegistry& Get();

  TabRegistry(const TabRegistry&) = delete;
  TabRegistry& operator=(const TabRegistry&) = delete;

  // The profile behind Jar::Main; set once the initial profile exists.
  void SetMainProfile(Profile* profile);

  // The bridge calls (quickie_bridge.h), with Chromium types.
  bool CreateTab(std::uint64_t tab, const GURL& url, std::optional<std::uint64_t> jar);
  bool ShowTab(std::uint64_t window, void* host_view, std::uint64_t tab);
  bool Navigate(std::uint64_t tab, NavigationKind kind, const GURL& url);
  bool CloseTab(std::uint64_t tab);
  bool AdoptTab(std::uint64_t page, std::uint64_t tab);
  bool DestroyJar(std::uint64_t jar);

  // For the BrowserWindow factory (//quickie/browser_window/window_factory.mm): the window for a
  // Browser the registry is creating, or for a normal or popup Browser Chromium creates on its
  // own (window.open, chrome.windows.create, the dock menu), which Quickie also hosts and whose
  // pages it reports as opened. Returns nullptr for the rest (DevTools, apps, picture in
  // picture), which keep Chromium's own windows.
  std::unique_ptr<QuickieBrowserWindow> MaybeCreateWindow(BrowserWindowInterface* browser);

  // QuickieBrowserWindow reports its Browser is going away.
  void OnWindowDestroyed(QuickieBrowserWindow* window);

  // TabStripModelObserver:
  void OnTabStripModelChanged(TabStripModel* tab_strip_model,
                              const TabStripModelChange& change,
                              const TabStripSelectionChange& selection) override;

 private:
  friend class base::NoDestructor<TabRegistry>;
  class PageObserver;
  struct Page;

  // A shell window and the profile of the Browser that hosts its pages from that profile.
  using BrowserKey = std::pair<std::uint64_t, Profile*>;

  TabRegistry();
  ~TabRegistry() override;

  Profile* ProfileForJar(std::optional<std::uint64_t> jar);
  BrowserWindowInterface* BrowserFor(std::uint64_t window, Profile* profile, void* host_view);
  Page* FindPage(std::uint64_t tab);
  // Every Browser with a QuickieBrowserWindow: bound to a shell window or not.
  std::vector<BrowserWindowInterface*> AllBrowsers() const;
  std::optional<std::uint64_t> TabIdOf(const content::WebContents* contents) const;
  // A page entered a tab strip: ours being shown or moved, or one Chromium opened.
  void OnPageInserted(TabStripModel* tab_strip_model,
                      content::WebContents* contents,
                      int index);
  // Chromium destroyed a page on its own.
  void Forget(const content::WebContents* contents);

  raw_ptr<Profile> main_profile_ = nullptr;
  // Throwaway jar id -> its off-the-record profile.
  std::map<std::uint64_t, raw_ptr<Profile>> jars_;
  // Tab id -> page.
  std::map<std::uint64_t, std::unique_ptr<Page>> pages_;
  // Pages Chromium opened, by the handle reported in PageOpenedTab, until Rust adopts them.
  std::map<std::uint64_t, std::unique_ptr<Page>> opened_;
  std::uint64_t next_opened_handle_ = 1;
  std::map<BrowserKey, raw_ptr<BrowserWindowInterface>> browsers_;
  // Browsers Chromium created on its own (MaybeCreateWindow), not yet tied to a shell window.
  std::vector<raw_ptr<BrowserWindowInterface>> unbound_browsers_;
  // Set while the registry creates a Browser, so the factory knows its shell window.
  struct PendingWindow {
    std::uint64_t window;
    // An NSView*, not owned; the shell keeps it alive (quickie_bridge.h, ShowTab).
    RAW_PTR_EXCLUSION void* host_view;
  };
  std::optional<PendingWindow> pending_window_;
};

}  // namespace quickie

#endif  // QUICKIE_BRIDGE_TAB_REGISTRY_H_
