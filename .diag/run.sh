#!/bin/bash
# EXPERIMENT (never merged): one traced run of the video stutter, for `fork` or `system`.
#
#   .diag/run.sh fork   [url]
#   .diag/run.sh system [url]
#
# Builds the debug app on our WebKit fork (`fork`) or the system WebKit (`system`), starts it
# with every trace on, walks you through the same phases each time, and packs everything into
# .diag/out/<variant>-<time>.tar.gz. Quit Safari first (its helpers share names).
# Asks for sudo once: the unified log and the helpers' task info need it.
set -euo pipefail

variant=${1:?usage: .diag/run.sh fork|system [url]}
url=${2:-https://www.youtube.com/watch?v=Y6myEUDtYzI}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
stamp=$(date +%Y%m%d-%H%M%S)
out=$root/.diag/out/$variant-$stamp
mkdir -p "$out"
app=target/debug/Quickie.app
bin=$app/Contents/MacOS/quickie
domain=dev.quickie.Quickie

ms() { python3 -c 'import time; print(f"{time.time()*1000:.2f}")'; }
mark() { printf '{"rx":%s,"mark":"%s"}\n' "$(ms)" "$1" >>"$out/markers.jsonl"; }
step() {
  printf '\n\033[1m%s\033[0m\n' "$1"
  read -r -p "  press Enter when done… " _
}

echo "== building ($variant)"
swiftc -O .diag/framewatch.swift -o .diag/framewatch
case $variant in
  fork) mise run webkit-fetch >/dev/null; cargo xtask bundle --webkit .build/webkit-frameworks ;;
  system) cargo xtask bundle ;;
  *) echo "fork or system" >&2; exit 2 ;;
esac

echo "== machine"
{
  sw_vers
  sysctl -n hw.model
  system_profiler SPDisplaysDataType 2>/dev/null | sed -n '/Displays:/,$p'
  pmset -g | grep -i -E 'lowpowermode|powermode' || true
  git log --oneline -1
  echo "variant=$variant url=$url"
  otool -L "$bin" | grep -i webkit || true
} >"$out/meta.txt" 2>&1

# WebKit's own media, compositing and display-link logging, into the unified log.
defaults write "$domain" WebCoreLogging -string "Media=debug,MediaSource=debug,Compositing=debug"
defaults write "$domain" WebKit2Logging -string "Media=debug,RemoteLayerTree=debug,DisplayLink=debug,Process=debug,ProcessSuspension=debug"

sudo -v
pids=()
cleanup() {
  for pid in "${pids[@]}"; do sudo kill "$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true; done
  defaults delete "$domain" WebCoreLogging 2>/dev/null || true
  defaults delete "$domain" WebKit2Logging 2>/dev/null || true
}
trap cleanup EXIT

# shellcheck disable=SC2024 # the file is meant to be ours, not root's
sudo log stream --style ndjson --level debug --predicate \
  'subsystem BEGINSWITH "com.apple.WebKit" OR subsystem BEGINSWITH "com.apple.coremedia" OR subsystem BEGINSWITH "com.apple.avf" OR subsystem == "com.apple.VideoToolbox" OR process BEGINSWITH "com.apple.WebKit" OR process == "quickie" OR (process == "WindowServer" AND (eventMessage CONTAINS[c] "refresh" OR eventMessage CONTAINS[c] "frame rate" OR eventMessage CONTAINS[c] "promotion" OR eventMessage CONTAINS[c] "quickie"))' \
  >"$out/log.ndjson" 2>/dev/null &
pids+=($!)

QUICKIE_TRACE=$out/trace.jsonl "$bin" "$url" >"$out/app.stdout" 2>"$out/app.stderr" &
app_pid=$!
pids+=("$app_pid")
mark start

# The helpers, twice a second: priority, nice, state, CPU; their task policy every 2 s.
(
  n=0
  while kill -0 "$app_pid" 2>/dev/null; do
    helpers=$(pgrep -f 'com\.apple\.WebKit\.(WebContent|GPU|Networking)' | tr '\n' ',' | sed 's/,$//')
    t=$(ms)
    [ -n "$helpers" ] && ps -o pid=,pri=,ni=,stat=,%cpu=,comm= -p "$helpers,$app_pid" | sed "s|^|$t |" >>"$out/ps.txt"
    if [ $((n % 4)) -eq 0 ] && [ -n "$helpers" ]; then
      for pid in ${helpers//,/ }; do
        { echo "== $t $pid"; sudo taskinfo "$pid" 2>&1 | grep -i -E 'process|role|qos|darwinbg|latency|throughput|suspend|policy|importance|boost'; } >>"$out/taskinfo.txt"
      done
    fi
    n=$((n + 1))
    sleep 0.5
  done
) &
pids+=($!)

step "1. The page opened in Quickie. Start the video (with sound). Keep the window NOT full screen."
sleep 2
.diag/framewatch >"$out/frames.jsonl" 2>"$out/framewatch.stderr" &
pids+=($!)
mark windowed
step "2. Watch it windowed for ~20 s (count stutters you see, if any)."
mark fullscreen-enter
step "3. Make the window full screen (green button or ⌃⌘F). Do not touch anything for ~40 s. Count the stutters."
mark settings-open
step "4. Still full screen: open Settings (⌘,) and leave it over the video for ~20 s."
mark settings-closed
step "5. Close Settings, click the video area once (window active again), watch ~30 s."
mark fullscreen-exit
step "6. Leave full screen. Watch ~15 s windowed."
mark end
read -r -p "How many stutters did you see in step 3 (full screen, active)? " count
printf '{"rx":%s,"mark":"user-count-step3","count":"%s"}\n' "$(ms)" "$count" >>"$out/markers.jsonl"

kill "$app_pid" 2>/dev/null || true
sleep 1
cleanup
trap - EXIT
tar -czf "$out.tar.gz" -C "$(dirname "$out")" "$(basename "$out")"
echo
echo "== done: $out.tar.gz ($(du -h "$out.tar.gz" | cut -f1))"
