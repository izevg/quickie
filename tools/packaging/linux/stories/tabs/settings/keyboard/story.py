"""Record shortcuts in GTK Settings and verify actual browser commands."""

import json
from pathlib import Path
import time

from PIL import ImageChops

from ..story import named_window, painted_general

FIXTURE = "tabs/settings/keyboard/fixture.html"
PAGE = (27, 69, 94)
RELOAD = "quickie.tab.reload"
FOOTER = (21, 389, 575, 441)
SOURCE = (456, 145, 566, 174)
REJECTED = (
    {"key": "ctrl+t", "command": "quickie.tab.hardReload"},
    {"key": "ctrl+ж", "command": "quickie.page.find"},
)


def region(story, image, box):
    return image.crop(tuple(value * story.scale for value in box))


def changed(story, window, previous, box, description):
    def visible():
        image = story.picture(window)
        difference = ImageChops.difference(region(story, previous, box), region(story, image, box))
        ink = difference.convert("L").point(lambda value: 255 if value >= 24 else 0).getbbox()
        return image if ink and ink[2] - ink[0] > 12 * story.scale else None

    return story.wait(visible, description)


def page(story, browser):
    def painted():
        image = story.picture(browser)
        return image if image.getpixel((16 * story.scale, 140 * story.scale)) == PAGE else None

    return story.wait(painted, "the live keyboard fixture")


def preferences(story, browser):
    story.focus(browser)
    story.key("ctrl+comma")
    window = named_window(story, "Quickie Settings")
    before = painted_general(story, window)
    story.click(window, before.width - 68 * story.scale, 28 * story.scale)
    return window, changed(story, window, before, (20, 54, 410, 126), "the Keyboard pane")


def close_preferences(story, window):
    story.key("Escape")
    story.wait(lambda: window not in story.clients(), "Settings to close")


def search(story, window, query):
    before = story.picture(window)
    story.key("ctrl+f", "ctrl+a")
    story.paste(query)
    return changed(story, window, before, (24, 123, 550, 360), f"the filtered {query!r} rows")


def record(story, window, keyboard_path, start):
    table = search(story, window, "reload tab")
    story.key("Down", "Return")
    recording = changed(story, window, table, FOOTER, "the recording instructions")
    story.save(recording, "keyboard-recording")
    story.key("ctrl+t")
    conflict = changed(story, window, recording, FOOTER, "the shortcut conflict")
    story.save(conflict, "keyboard-conflict")
    story.key("Return")
    assert keyboard_path.read_bytes() == start
    assert len(story.clients()) == 2, "a recorded shortcut was sent to the browser"

    story.key("shift+f")
    typing = changed(story, window, conflict, FOOTER, "the non-shortcut typing refusal")
    story.save(typing, "keyboard-typing")
    story.key("Return")
    assert keyboard_path.read_bytes() == start

    story.key("ctrl+shift+e")
    ready = changed(story, window, typing, FOOTER, "the valid recorded shortcut")
    story.save(ready, "keyboard-recorded")
    story.key("Return")
    story.wait(
        lambda: {"key": "ctrl+shift+e", "command": RELOAD}
        in json.loads(keyboard_path.read_text()),
        "the recorded shortcut to persist in keybindings.json",
    )
    changed(story, window, ready, SOURCE, "the effective binding to show its user source")
    assert all(entry in json.loads(keyboard_path.read_text()) for entry in REJECTED)


def reload_once(story, shortcut):
    count = story.requests(FIXTURE)
    story.key(shortcut)
    story.wait(lambda: story.requests(FIXTURE) > count, f"{shortcut} to reload the live page")


def stays_loaded(story, shortcut):
    count = story.requests(FIXTURE)
    story.key(shortcut)
    deadline = time.monotonic() + 0.45
    story.wait(lambda: time.monotonic() >= deadline, "the compositor to deliver the shortcut")
    assert story.requests(FIXTURE) == count, f"{shortcut} unexpectedly reloaded the page"


def palette(story, browser):
    before = story.picture(browser)
    story.key("ctrl+shift+p")
    shown = changed(story, browser, before, (420, 146, 794, 370), "the native command palette")
    story.paste("reload tab")
    image = changed(story, browser, shown, (420, 178, 850, 370), "the filtered Reload command")
    story.key("Escape")
    page(story, browser)
    return image


def conditional_shortcut(story, keyboard_path):
    binding = {
        "key": "ctrl+shift+e",
        "command": RELOAD,
        "when": "pageFocused && !inputFocused",
    }
    keyboard_path.write_text(json.dumps([*REJECTED, binding]) + "\n")
    browser = story.open()
    page(story, browser)
    story.click(browser, 240 * story.scale, 194 * story.scale)
    story.wait(lambda: story.requests(f"{FIXTURE}?focus=input") > 0, "the page input to take focus")
    stays_loaded(story, "ctrl+shift+e")
    story.click(browser, 56 * story.scale, 322 * story.scale)
    story.wait(lambda: story.requests(f"{FIXTURE}?focus=page") > 0, "the page input to lose focus")
    reload_once(story, "ctrl+shift+e")
    story.close(browser)


def run(story):
    # Arrange a real file that contains a conflict.
    keyboard_path = Path(story.environment["XDG_CONFIG_HOME"]) / "quickie" / "keybindings.json"
    keyboard_path.parent.mkdir()
    keyboard_path.write_text(json.dumps(REJECTED[:1]) + "\n")
    browser = story.open(FIXTURE)
    page(story, browser)
    original_hint = palette(story, browser)

    # Open Keyboard and use the actual editor. The file remains intact on each refusal.
    window, full = preferences(story, browser)
    story.save(full, "keyboard")
    keyboard_path.write_text(json.dumps(REJECTED) + "\n")
    start = keyboard_path.read_bytes()
    story.click(window, full.width - 58 * story.scale, 71 * story.scale)
    story.move_pointer(window, 4, 4)
    story.save(
        changed(story, window, full, FOOTER, "the reloaded file's problems"),
        "keyboard-problems",
    )
    refused = search(story, window, "without cache")
    story.key("Down", "Down")
    story.save(changed(story, window, refused, FOOTER, "the rejected file entry's reason"),
               "keyboard-refused")
    record(story, window, keyboard_path, start)
    close_preferences(story, window)
    story.focus(browser)
    stays_loaded(story, "ctrl+r")
    reload_once(story, "ctrl+shift+e")
    rebound_hint = palette(story, browser)
    old_hint = region(story, original_hint, (755, 175, 873, 310))
    new_hint = region(story, rebound_hint, (755, 175, 873, 310))
    assert ImageChops.difference(old_hint, new_hint).getbbox(), "the palette kept the old shortcut"

    # Restart with the same temporary profile, then reset to the platform default.
    story.close(browser)
    browser = story.open()
    page(story, browser)
    stays_loaded(story, "ctrl+r")
    reload_once(story, "ctrl+shift+e")
    window, _ = preferences(story, browser)
    reset_rows = search(story, window, "reload tab")
    story.key("Down")
    before_reset = changed(story, window, reset_rows, SOURCE, "the override to be selected")
    story.key("BackSpace")
    story.wait(
        lambda: all(entry["command"].removeprefix("-") != RELOAD
                    for entry in json.loads(keyboard_path.read_text())),
        "Reset to remove the Reload override",
    )
    changed(story, window, before_reset, SOURCE, "the default binding to return")
    close_preferences(story, window)
    story.focus(browser)
    reload_once(story, "ctrl+r")
    stays_loaded(story, "ctrl+shift+e")
    story.close(browser)
    conditional_shortcut(story, keyboard_path)
