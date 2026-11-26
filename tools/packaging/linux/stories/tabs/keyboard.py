"""Drive the switcher with a held Control modifier."""

from contextlib import contextmanager
import time


@contextmanager
def control_tab(story, actions=(), hold_ms=1100):
    desktop = story.desktop
    if story.environment.get("WAYLAND_DISPLAY"):
        keyboard = desktop.keyboard
        started = time.monotonic()
        keyboard.hold_modifier("ctrl")
        try:
            keyboard.tap("Tab")
            if actions:
                time.sleep(0.27)
                keyboard.tap(*actions)
            started = time.monotonic()
            yield
        finally:
            remaining = hold_ms / 1000.0 - (time.monotonic() - started)
            if remaining > 0:
                time.sleep(remaining)
            keyboard.release_modifier("ctrl")
            # Deliver the cleared Wayland modifier state to GTK's key controller.
            keyboard.tap("Shift_L")
    else:
        desktop.xd("keydown", "Control_L", "key", "Tab")
        try:
            if actions:
                time.sleep(0.27)
                desktop.xd("key", *actions)
            yield
        finally:
            desktop.xd("keyup", "Control_L")
