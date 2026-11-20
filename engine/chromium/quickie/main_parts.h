// Quickie's part of browser startup: a ChromeBrowserMainExtraParts that
// engine/chromium/patches/startup/browser-main-parts.patch adds to ChromeBrowserMainParts.

#ifndef QUICKIE_MAIN_PARTS_H_
#define QUICKIE_MAIN_PARTS_H_

#include <memory>

#include "chrome/browser/chrome_browser_main_extra_parts.h"

namespace quickie {

class MainParts : public ChromeBrowserMainExtraParts {
 public:
  MainParts();
  MainParts(const MainParts&) = delete;
  MainParts& operator=(const MainParts&) = delete;
  ~MainParts() override;

  // ChromeBrowserMainExtraParts (chrome/browser/chrome_browser_main_extra_parts.h):
  void PreEarlyInitialization() override;
  void PostProfileInit(Profile* profile, bool is_initial_profile) override;
  void PostBrowserStart() override;
};

// Called by the patched ChromeBrowserMainParts::Create; replaces the weak default there.
std::unique_ptr<ChromeBrowserMainExtraParts> CreateBrowserMainExtraParts();

}  // namespace quickie

#endif  // QUICKIE_MAIN_PARTS_H_
