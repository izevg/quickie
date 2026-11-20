// See tab_registry.h. Chromium APIs used here were checked against 156.0.8078.109; the header
// each comes from is named next to its first use.

#include "quickie/bridge/tab_registry.h"

#include <string>
#include <utility>

#include "base/functional/bind.h"
#include "base/location.h"
#include "base/logging.h"
#include "base/no_destructor.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/sequenced_task_runner.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/profiles/profile_destroyer.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/browser_window/public/create_browser_window.h"
#include "chrome/browser/ui/tabs/tab_enums.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/navigation_handle.h"
#include "content/public/browser/reload_type.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "quickie/browser_window/quickie_browser_window.h"
#include "quickie_crates/adapters/engine-chromium/src/ffi.rs.h"
#include "ui/base/page_transition_types.h"
#include "url/gurl.h"

namespace quickie {

namespace {

void Report(EngineEventKind kind,
            std::uint64_t tab,
            std::optional<std::uint64_t> opener,
            const std::string& text) {
  EngineEvent event{
      kind,
      tab,
      opener.has_value(),
      opener.value_or(0),
      // Titles come from UTF-16 and URLs are ASCII, but never trust it with a panic.
      rust::String::lossy(text),
  };
  OnEngineEvent(event);
}

}  // namespace

// Reports a page's navigations and title to Rust under its tab id; silent until the page has
// one (a page Chromium opened, before AdoptTab).
// content/public/browser/web_contents_observer.h
class TabRegistry::PageObserver : public content::WebContentsObserver {
 public:
  PageObserver(content::WebContents* contents, std::optional<std::uint64_t> tab)
      : content::WebContentsObserver(contents), tab_(tab) {}

  void set_tab(std::uint64_t tab) { tab_ = tab; }

  // content/public/browser/navigation_handle.h
  void DidFinishNavigation(content::NavigationHandle* handle) override {
    if (!tab_ || !handle->IsInPrimaryMainFrame() || !handle->HasCommitted()) {
      return;
    }
    Report(EngineEventKind::NavigationCommitted, *tab_, std::nullopt,
           handle->GetURL().spec());
  }

  void TitleWasSet(content::NavigationEntry* entry) override {
    if (!tab_) {
      return;
    }
    Report(EngineEventKind::TitleChanged, *tab_, std::nullopt,
           base::UTF16ToUTF8(web_contents()->GetTitle()));
  }

  void WebContentsDestroyed() override {
    // Chromium destroyed the page itself (window.close(), the Browser closing).
    // TODO(verify): Rust is not told yet; P1 needs an event for it (see engine/chromium/README.md).
    TabRegistry::Get().Forget(web_contents());
  }

 private:
  std::optional<std::uint64_t> tab_;
};

struct TabRegistry::Page {
  // Owned only while detached (created, never shown); a TabStripModel owns it afterwards.
  std::unique_ptr<content::WebContents> detached;
  raw_ptr<content::WebContents> contents;
  raw_ptr<Profile> profile;
  // Last, so it stops observing before `detached` is destroyed.
  std::unique_ptr<PageObserver> observer;
};

// static
TabRegistry& TabRegistry::Get() {
  static base::NoDestructor<TabRegistry> registry;
  return *registry;
}

TabRegistry::TabRegistry() = default;
TabRegistry::~TabRegistry() = default;

void TabRegistry::SetMainProfile(Profile* profile) {
  main_profile_ = profile;
}

Profile* TabRegistry::ProfileForJar(std::optional<std::uint64_t> jar) {
  if (!main_profile_) {
    LOG(ERROR) << "quickie: no main profile yet";
    return nullptr;
  }
  if (!jar) {
    return main_profile_;
  }
  auto it = jars_.find(*jar);
  if (it != jars_.end()) {
    return it->second;
  }
  // chrome/browser/profiles/profile.h; see tab_registry.h for why the DevTools id.
  Profile* otr = main_profile_->GetOffTheRecordProfile(
      Profile::OTRProfileID::CreateUniqueForDevTools(), /*create_if_needed=*/true);
  jars_.emplace(*jar, otr);
  return otr;
}

bool TabRegistry::CreateTab(std::uint64_t tab,
                            const GURL& url,
                            std::optional<std::uint64_t> jar) {
  if (pages_.contains(tab) || !url.is_valid()) {
    return false;
  }
  Profile* profile = ProfileForJar(jar);
  if (!profile) {
    return false;
  }
  // content/public/browser/web_contents.h
  auto page = std::make_unique<Page>();
  page->detached =
      content::WebContents::Create(content::WebContents::CreateParams(profile));
  page->contents = page->detached.get();
  page->profile = profile;
  page->observer = std::make_unique<PageObserver>(page->contents, tab);
  // content/public/browser/navigation_controller.h
  content::NavigationController::LoadURLParams params(url);
  params.transition_type = ui::PAGE_TRANSITION_AUTO_TOPLEVEL;
  page->contents->GetController().LoadURLWithParams(params);
  pages_.emplace(tab, std::move(page));
  return true;
}

BrowserWindowInterface* TabRegistry::BrowserFor(std::uint64_t window,
                                                Profile* profile,
                                                void* host_view) {
  BrowserKey key{window, profile};
  auto it = browsers_.find(key);
  if (it != browsers_.end()) {
    return it->second;
  }
  // chrome/browser/ui/browser_window/public/create_browser_window.h. The factory override asks
  // MaybeCreateWindow, which reads pending_window_.
  pending_window_ = PendingWindow{window, host_view};
  BrowserWindowInterface* browser = CreateBrowserWindow(BrowserWindowCreateParams(
      BrowserWindowInterface::TYPE_NORMAL, *profile, /*from_user_gesture=*/true));
  pending_window_.reset();
  if (!browser) {
    return nullptr;
  }
  browser->GetTabStripModel()->AddObserver(this);
  browsers_.emplace(key, browser);
  // Whoever creates a Browser shows its window; for ours that lets pages draw.
  if (QuickieBrowserWindow* hosted = QuickieBrowserWindow::FromBrowser(browser)) {
    hosted->Show();
  }
  return browser;
}

bool TabRegistry::ShowTab(std::uint64_t window, void* host_view, std::uint64_t tab) {
  Page* page = FindPage(tab);
  if (!page || !host_view) {
    return false;
  }
  BrowserWindowInterface* browser = BrowserFor(window, page->profile, host_view);
  if (!browser) {
    return false;
  }
  TabStripModel* target = browser->GetTabStripModel();

  if (!page->detached) {
    // In a tab strip already: if another one, detach it from there (that Browser closes once
    // empty, chrome/browser/ui/browser.cc TabStripEmpty).
    for (BrowserWindowInterface* other : AllBrowsers()) {
      TabStripModel* source = other->GetTabStripModel();
      int index = source->GetIndexOfWebContents(page->contents);
      if (index == TabStripModel::kNoTab || source == target) {
        continue;
      }
      page->detached = source->DetachWebContentsAtForInsertion(index);
      break;
    }
  }
  if (page->detached) {
    // chrome/browser/ui/tabs/tab_strip_model.h, tab_enums.h
    target->InsertWebContentsAt(target->count(), std::move(page->detached),
                                AddTabTypes::ADD_ACTIVE);
  } else {
    int index = target->GetIndexOfWebContents(page->contents);
    if (index == TabStripModel::kNoTab) {
      return false;
    }
    target->ActivateTabAt(index);
  }

  // Only this Browser's page fills the shell window; its other Browsers (other jars) step back.
  for (const auto& [key, other] : browsers_) {
    if (key.first != window) {
      continue;
    }
    if (QuickieBrowserWindow* hosted = QuickieBrowserWindow::FromBrowser(other)) {
      hosted->SetAttached(other == browser);
    }
  }
  return true;
}

bool TabRegistry::Navigate(std::uint64_t tab, NavigationKind kind, const GURL& url) {
  Page* page = FindPage(tab);
  if (!page) {
    return false;
  }
  content::NavigationController& controller = page->contents->GetController();
  switch (kind) {
    case NavigationKind::Load: {
      if (!url.is_valid()) {
        return false;
      }
      content::NavigationController::LoadURLParams params(url);
      params.transition_type = ui::PAGE_TRANSITION_TYPED;
      return !!controller.LoadURLWithParams(params);
    }
    case NavigationKind::Back:
      if (!controller.CanGoBack()) {
        return false;
      }
      controller.GoBack();
      return true;
    case NavigationKind::Forward:
      if (!controller.CanGoForward()) {
        return false;
      }
      controller.GoForward();
      return true;
    case NavigationKind::Reload:
      controller.Reload(content::ReloadType::NORMAL, /*check_for_repost=*/true);
      return true;
    case NavigationKind::HardReload:
      controller.Reload(content::ReloadType::BYPASSING_CACHE, /*check_for_repost=*/true);
      return true;
    case NavigationKind::Stop:
      page->contents->Stop();
      return true;
  }
  return false;
}

bool TabRegistry::CloseTab(std::uint64_t tab) {
  auto it = pages_.find(tab);
  if (it == pages_.end()) {
    return false;
  }
  std::unique_ptr<Page> page = std::move(it->second);
  pages_.erase(it);
  page->observer.reset();
  if (page->detached) {
    return true;  // Destroyed with `page`.
  }
  for (BrowserWindowInterface* browser : AllBrowsers()) {
    TabStripModel* strip = browser->GetTabStripModel();
    int index = strip->GetIndexOfWebContents(page->contents);
    if (index != TabStripModel::kNoTab) {
      // Runs unload handlers; the WebContents goes away asynchronously.
      strip->CloseWebContentsAt(index, TabCloseTypes::CLOSE_NONE);
      return true;
    }
  }
  return false;
}

bool TabRegistry::AdoptTab(std::uint64_t page_handle, std::uint64_t tab) {
  auto it = opened_.find(page_handle);
  if (it == opened_.end() || pages_.contains(tab)) {
    return false;
  }
  std::unique_ptr<Page> page = std::move(it->second);
  opened_.erase(it);
  page->observer->set_tab(tab);
  // What happened before adoption was not reported; report the current state once.
  if (const GURL& url = page->contents->GetLastCommittedURL(); url.is_valid()) {
    Report(EngineEventKind::NavigationCommitted, tab, std::nullopt, url.spec());
  }
  Report(EngineEventKind::TitleChanged, tab, std::nullopt,
         base::UTF16ToUTF8(page->contents->GetTitle()));
  pages_.emplace(tab, std::move(page));
  return true;
}

bool TabRegistry::DestroyJar(std::uint64_t jar) {
  auto it = jars_.find(jar);
  if (it == jars_.end()) {
    return false;
  }
  Profile* profile = it->second;
  for (const auto& [id, page] : pages_) {
    if (page->profile == profile) {
      LOG(ERROR) << "quickie: jar " << jar << " still has tab " << id;
      return false;
    }
  }
  // Pages Chromium opened in the jar and Rust never adopted die with it.
  std::erase_if(opened_, [profile](const auto& entry) {
    return entry.second->profile == profile;
  });
  std::erase_if(browsers_,
                [profile](const auto& entry) { return entry.first.second == profile; });
  jars_.erase(it);
  // chrome/browser/profiles/profile_destroyer.h: waits for the renderers to go first.
  ProfileDestroyer::DestroyOTRProfileWhenAppropriate(profile);
  return true;
}

std::unique_ptr<QuickieBrowserWindow> TabRegistry::MaybeCreateWindow(
    BrowserWindowInterface* browser) {
  if (pending_window_) {
    return std::make_unique<QuickieBrowserWindow>(browser, pending_window_->window,
                                                  pending_window_->host_view);
  }
  const auto type = browser->GetType();
  if (type != BrowserWindowInterface::TYPE_NORMAL &&
      type != BrowserWindowInterface::TYPE_POPUP) {
    return nullptr;
  }
  // A Browser Chromium made on its own. It gets a window with no shell window yet; its pages are
  // reported as opened, and the shell shows them where it wants (ShowTab moves them).
  // TODO(verify): Chromium may insert the first tab before the factory returns, that is before
  // this observer is added; the posted scan below covers that.
  browser->GetTabStripModel()->AddObserver(this);
  unbound_browsers_.push_back(browser);
  base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce(
                     [](base::WeakPtr<BrowserWindowInterface> weak) {
                       if (!weak) {
                         return;
                       }
                       TabStripModel* strip = weak->GetTabStripModel();
                       for (int i = 0; i < strip->count(); ++i) {
                         TabRegistry::Get().OnPageInserted(strip, strip->GetWebContentsAt(i), i);
                       }
                     },
                     browser->GetWeakPtr()));
  return std::make_unique<QuickieBrowserWindow>(browser, /*window=*/0, /*host_view=*/nullptr);
}

void TabRegistry::OnWindowDestroyed(QuickieBrowserWindow* window) {
  std::erase_if(browsers_, [window](const auto& entry) {
    return QuickieBrowserWindow::FromBrowser(entry.second) == window;
  });
  std::erase_if(unbound_browsers_, [window](BrowserWindowInterface* browser) {
    return QuickieBrowserWindow::FromBrowser(browser) == window;
  });
}

std::vector<BrowserWindowInterface*> TabRegistry::AllBrowsers() const {
  std::vector<BrowserWindowInterface*> all;
  for (const auto& [key, browser] : browsers_) {
    all.push_back(browser);
  }
  for (BrowserWindowInterface* browser : unbound_browsers_) {
    all.push_back(browser);
  }
  return all;
}

// chrome/browser/ui/tabs/tab_strip_model_observer.h
void TabRegistry::OnTabStripModelChanged(TabStripModel* tab_strip_model,
                                         const TabStripModelChange& change,
                                         const TabStripSelectionChange& selection) {
  if (change.type() != TabStripModelChange::kInserted) {
    return;
  }
  for (const auto& inserted : change.GetInsert()->contents) {
    OnPageInserted(tab_strip_model, inserted.contents, inserted.index);
  }
}

void TabRegistry::OnPageInserted(TabStripModel* tab_strip_model,
                                 content::WebContents* contents,
                                 int index) {
  if (!contents || TabIdOf(contents)) {
    return;  // One of ours, being shown or moved.
  }
  for (const auto& [handle, page] : opened_) {
    if (page->contents == contents) {
      return;  // Already reported.
    }
  }
  std::optional<std::uint64_t> opener;
  if (tabs::TabInterface* opener_tab = tab_strip_model->GetOpenerOfTabAt(index)) {
    opener = TabIdOf(opener_tab->GetContents());
  }
  auto page = std::make_unique<Page>();
  page->contents = contents;
  page->profile = Profile::FromBrowserContext(contents->GetBrowserContext());
  page->observer = std::make_unique<PageObserver>(contents, std::nullopt);
  std::uint64_t handle = next_opened_handle_++;
  opened_.emplace(handle, std::move(page));
  Report(EngineEventKind::PageOpenedTab, handle, opener,
         contents->GetVisibleURL().spec());
}

void TabRegistry::Forget(const content::WebContents* contents) {
  // Called from the page's own observer (WebContentsDestroyed): take the page out now, so no
  // later lookup finds it, but delete it (and so that observer) only after the callback returns.
  auto take = [contents](auto& pages) {
    for (auto it = pages.begin(); it != pages.end();) {
      if (it->second->contents == contents) {
        it->second->contents = nullptr;
        base::SequencedTaskRunner::GetCurrentDefault()->DeleteSoon(FROM_HERE,
                                                                   std::move(it->second));
        it = pages.erase(it);
      } else {
        ++it;
      }
    }
  };
  take(pages_);
  take(opened_);
}

TabRegistry::Page* TabRegistry::FindPage(std::uint64_t tab) {
  auto it = pages_.find(tab);
  return it == pages_.end() ? nullptr : it->second.get();
}

std::optional<std::uint64_t> TabRegistry::TabIdOf(const content::WebContents* contents) const {
  for (const auto& [id, page] : pages_) {
    if (page->contents == contents) {
      return id;
    }
  }
  return std::nullopt;
}

}  // namespace quickie
