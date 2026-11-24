"""Drive the switcher with real Control press and release events."""

from contextlib import contextmanager
import subprocess
import time


@contextmanager
def control_tab(story, actions=(), hold_ms=1100):
    if story.environment.get("WAYLAND_DISPLAY"):
        args = wayland_args(actions, hold_ms)
        with subprocess.Popen(args, env=story.environment) as keyboard:
            try:
                yield
            finally:
                assert keyboard.wait(timeout=10) == 0, "could not drive Wayland Control-Tab"
    else:
        desktop = story.desktop
        desktop.xd("keydown", "Control_L", "key", "Tab")
        try:
            if actions:
                time.sleep(0.27)
                desktop.xd("key", *actions)
            yield
        finally:
            desktop.xd("keyup", "Control_L")


def wayland_args(actions, hold_ms):
    args = ["wtype", "-M", "ctrl", "-k", "Tab"]
    if actions:
        args += ["-s", "270"]
    for action in actions:
        if action == "shift+Tab":
            args += ["-M", "shift", "-k", "Tab", "-m", "shift"]
        else:
            args += ["-k", action]
    # The Wayland virtual keyboard reports modifiers separately from physical keys.
    # A harmless Shift key delivers the cleared state to GTK after Ctrl release.
    return args + ["-s", str(hold_ms), "-m", "ctrl", "-k", "Shift_L"]
