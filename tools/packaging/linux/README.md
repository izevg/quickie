# Quickie for Linux

The Linux preview is a native GTK4 browser with the same pinned WebKit fork as the macOS
build. Wayland is the primary Linux display target. X11 is supported for compatibility.
A fresh profile opens an empty window and makes its first navigation request when you enter an
address or pass one explicitly. Use **Ctrl+L** for the address overlay, **Ctrl+Shift+P** or
**Ctrl+K** for the command palette, and **Ctrl+P** to search open tabs and pages visited in the
current window. In the address overlay, **Tab** completes a matching visited address when the
shortcut is shown. The arrow keys move through results, **Enter** activates a result, and **Esc**
returns focus to the page.

Use **Ctrl+F** to find text on the page. **Enter** or **Ctrl+G** selects the next match,
and **Shift+Enter** or **Ctrl+Shift+G** selects the previous one. The selection wraps at both
ends. **Esc** closes the find panel and returns focus to the page.

Use **Ctrl+R** to reload, **Alt+Left** and **Alt+Right** for history, and **F12** or
**Ctrl+Shift+I** to toggle Web Inspector. **Ctrl+,** opens Settings, with access
to Software Updates. **Ctrl+J** opens Downloads.

Use **Ctrl+T** to open a tab, **Ctrl+W** to close it, and **Ctrl+Shift+T** to restore
it from the trash. Hold **Ctrl** and press **Tab** to switch between recent tabs.
Open the tab overview through the command palette or `Ctrl+Shift+\`. Each profile
keeps its browsing session across launches.

Open Shields with **Ctrl+Alt+S**. Enable filter lists in Shields settings and
manage exceptions for the current site. Open **Agent** from the command palette
to ask about the source page. The **Agent Settings** command configures the
endpoint and stores its key in the desktop keyring.

## Packages

All packages are for x86-64. The Debian package targets Ubuntu 24.04 LTS, and the RPM targets
Fedora 43. The AppImage carries Quickie's WebKitGTK libraries, helper processes and resources,
and uses the host's GTK4, media libraries, and system libraries. Its tested runtime baseline is
Ubuntu 24.04 LTS with the runtime dependencies of the matching Debian package. Launch it with
`./Quickie-0.2.0-x86_64.AppImage` (mark it executable first). If FUSE is unavailable, the
AppImage runtime supports `--appimage-extract-and-run`.

The WebKit subprocess sandbox uses Bubblewrap and xdg-dbus-proxy. On Ubuntu systems with
AppArmor user namespace restrictions, install and load the distribution's
`bwrap-userns-restrict` profile from the `apparmor-profiles` package if it is required by
your point release. The sandbox remains enabled for both installed packages and AppImage.

The `linux packages` GitHub Actions workflow builds against the checked-in engine pin.
Pull requests to a release branch run it when they touch the Linux packaging or the engine pin.
To run it manually, select a Git ref and a numeric package version, then download the
`quickie-linux-ubuntu` and `quickie-linux-fedora` job artifacts. Each package contains the
exact engine revision and corresponding source URL in `share/doc/quickie` in its private
runtime. The workflow runs the installed browser and relocated AppImage against a local page
and checks that the web and network processes use the packaged paths. The web process must
also run in a separate PID namespace with an additional seccomp filter. WebKitGTK launches
its network process directly.
