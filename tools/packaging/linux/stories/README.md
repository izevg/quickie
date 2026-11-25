# GTK stories

These stories drive the compiled Quickie browser with native GTK and WebKitGTK. Wayland
runs in an isolated headless Sway session with XWayland disabled. The runner requires
native Wayland windows and uses compositor screenshots, keyboard input, and clipboard
selection. Each story starts a loopback fixture server and a fresh profile. It asserts
the expected surface and interactions before recording a pass. The output contains PNG
captures and a `passed.json` result with the selected backend for each group.

On Linux, install Sway, grim, wtype, wl-clipboard, D-Bus, and Python 3 with Pillow.
Run as an unprivileged user. Use an installed Quickie package or a locally built
`quickie-gtk` executable with a compatible WebKitGTK.

```sh
dbus-run-session -- python3 -B tools/packaging/linux/stories/driver.py \
  /usr/bin/quickie /tmp/quickie-gtk-stories --theme dark
```

Use `--group palette`, `--group find`, `--group inspector`, `--group dialogs`,
`--group tabs`, `--group privacy`, `--group agent`, `--group agent-providers`,
`--group media`, `--group downloads`, `--group settings`, or `--group keyboard`
for a targeted run.
Use `--scale 2` for HiDPI. For X11 compatibility, add `--backend x11`
and install Xvfb, Openbox, xdotool, `xprop`, and ImageMagick (`import`). Xvfb allocates
a free display automatically. Use `--display :98` to request a specific free display.
Keep generated screenshots outside the checkout. Record the distribution, GTK/WebKitGTK versions,
theme, and scale when comparing runs. Use the same runtime to compare pixels.

The privacy group exercises Shields with an installed or locally built browser.
To include the camera and microphone prompts, build a debug browser with
`cargo build -p quickie-linux --features gtk-story-media` and set
`QUICKIE_GTK_STORY_MEDIA=1` for the runner. WebKit supplies mock hardware to the
loopback fixture. The story drives the native permission prompt and checks the
returned media tracks.

Run `--group extensions` against a Quickie build with the fork's GTK extension
controller. It starts the browser with an isolated profile containing saved packages
and checks that only the enabled package with a matching grant runs. It also checks
that the same page opened in a private tab receives no ordinary-profile scripts.

New UI changes include AppKit and GTK stories in the same pull request. Match the existing
AppKit story ID where the interaction has the same meaning. Each GTK group owns its fixture
and a `run(story)` function. Assert the interaction outcome with fixture requests, visible
native state, or both. For asynchronous UI, wait for that state and save the frame that
passed the check with `story.save(image, name)`.
