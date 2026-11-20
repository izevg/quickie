#!/bin/sh
# Frees disk on a GitHub-hosted macOS runner before the WebKit build (.github/workflows/webkit.yml)
# by deleting what the image ships and the build never uses: every Xcode but the selected one,
# the simulator runtimes, the Android SDK, .NET, the Haskell toolchain, CodeQL and the browsers.
# Prints the free space after each step. Runs on GitHub-hosted runners only: it deletes
# applications with sudo.
set -eu

[ "${GITHUB_ACTIONS:-}" = true ] && [ "${RUNNER_ENVIRONMENT:-}" = github-hosted ] || {
  echo 'free-disk: runs on GitHub-hosted runners only' >&2
  exit 1
}

free() {
  df -Pk / | awk 'NR == 2 { printf "%.1f GB", $4 / 1048576 }'
}

# Removes the given paths in the background; `wait` below collects them.
remove() {
  for path in "$@"; do
    if [ -e "$path" ] || [ -L "$path" ]; then
      sudo rm -rf "$path" &
    fi
  done
}

printf 'free-disk: %s free before\n' "$(free)"

# Xcode: keep the selected one, by its resolved path (Xcode.app and Xcode_27.2.0.app are links).
selected=$(xcode-select -p)
keep=$(CDPATH='' cd -- "${selected%/Contents/Developer}" && pwd -P)
for app in /Applications/Xcode*.app; do
  [ -L "$app" ] && continue
  real=$(CDPATH='' cd -- "$app" && pwd -P)
  [ "$real" = "$keep" ] || remove "$app"
done
wait
# Links to the removed copies point nowhere now.
for app in /Applications/Xcode*.app; do
  [ -L "$app" ] && [ ! -e "$app" ] && sudo rm -f "$app"
done
printf 'free-disk: %s free, kept %s\n' "$(free)" "$keep"

# Simulators: devices, then runtimes (disk images registered with simctl, plus the bundled ones).
xcrun simctl shutdown all >/dev/null 2>&1 || true
xcrun simctl delete all >/dev/null 2>&1 || true
xcrun simctl runtime delete all >/dev/null 2>&1 || true
remove /Library/Developer/CoreSimulator/Profiles/Runtimes "$HOME/Library/Developer/CoreSimulator"
wait
printf 'free-disk: %s free, simulators removed\n' "$(free)"

# Toolchains and apps the WebKit build does not use. Python, Ruby and Perl stay: the build runs
# scripts in all three.
remove "${ANDROID_HOME:-$HOME/Library/Android/sdk}" "$HOME/Library/Android" \
  "$HOME/.dotnet" /usr/local/share/dotnet "$HOME/.ghcup" \
  "${RUNNER_TOOL_CACHE:-$HOME/hostedtoolcache}/CodeQL" \
  "/Applications/Google Chrome.app" "/Applications/Google Chrome for Testing.app" \
  /Applications/Firefox.app "/Applications/Microsoft Edge.app" \
  /usr/local/share/chromium /usr/local/share/edge_driver /usr/local/share/chromedriver-mac-arm64
wait
printf 'free-disk: %s free after\n' "$(free)"
