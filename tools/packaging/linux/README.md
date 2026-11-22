# Quickie for Linux

The Linux preview is a native GTK4 browser with the same pinned WebKit fork as the macOS
build. It opens an empty window and makes no navigation request until you enter an address or
pass one explicitly. Use **Ctrl+L** to open the address bar, **Esc** to dismiss it, **Ctrl+R**
to reload, **Alt+Left** and **Alt+Right** for history, and **Ctrl+Shift+I** to open Web Inspector.
The preview has one browsing window and the native WebKitGTK inspector. Session restoration,
Shields, the macOS inspector styling, and the agent API are outside this Linux preview.

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
runtime. The workflow runs the installed browser and relocated AppImage against a local page,
then checks whether the web and network processes use the packaged paths, a separate PID
namespace, and an added seccomp filter.
