// See main_parts.h.

#include "quickie/main_parts.h"

#include "base/command_line.h"
#include "chrome/common/chrome_switches.h"
#include "quickie/bridge/tab_registry.h"
#include "quickie_crates/adapters/engine-chromium/src/ffi.rs.h"

namespace quickie {

MainParts::MainParts() = default;
MainParts::~MainParts() = default;

void MainParts::PreEarlyInitialization() {
  // The Rust shell opens windows and restores the session from shell.sqlite (architecture
  // section 4.3); Chromium must not open a startup window or restore its own session.
  // chrome/browser/ui/startup/startup_browser_creator.cc reads this switch later, in
  // PreMainMessageLoopRun.
  // TODO(verify): the macOS app controller (chrome/browser/app_controller_mac.mm) still opens a
  // window on Dock-icon reopen; that Browser gets a QuickieBrowserWindow with no shell window.
  base::CommandLine::ForCurrentProcess()->AppendSwitch(switches::kNoStartupWindow);
}

void MainParts::PostProfileInit(Profile* profile, bool is_initial_profile) {
  if (is_initial_profile) {
    TabRegistry::Get().SetMainProfile(profile);
  }
}

void MainParts::PostBrowserStart() {
  // The browser process is up, the UI loop is about to run: the shell takes over the main
  // thread from here (cxx bridge, crates/adapters/engine-chromium/src/ffi.rs).
  ShellStart();
}

std::unique_ptr<ChromeBrowserMainExtraParts> CreateBrowserMainExtraParts() {
  return std::make_unique<MainParts>();
}

}  // namespace quickie
