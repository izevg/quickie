"""Exercise General in the browser and observe its effects on the isolated desktop."""

from pathlib import Path
import sqlite3
from subprocess import check_output

from PIL import ImageChops

FIXTURE = "tabs/settings/fixture.html"
PAGE = (42, 98, 128)
SCHEMES = ("x-scheme-handler/http", "x-scheme-handler/https")
DEFAULT_Y, DOWNLOADS_Y, INSET_Y, UPDATES_Y = 116, 209, 310, 412


def configure_desktop(story):
    config = Path(story.environment["XDG_CONFIG_HOME"])
    data = Path(story.environment["XDG_DATA_HOME"])
    system_data = data.parent / "system-data"
    system_config = config.parent / "system-config"
    system_data.mkdir()
    system_config.mkdir()
    data_roots = story.environment.get("XDG_DATA_DIRS", "/usr/local/share:/usr/share").split(":")
    for asset in ("glib-2.0", "icons", "themes", "mime"):
        for root in data_roots:
            source = Path(root) / asset
            if source.is_dir():
                (system_data / asset).symlink_to(source, target_is_directory=True)
                break
    story.environment.update(
        XDG_DATA_DIRS=str(system_data), XDG_CONFIG_DIRS=str(system_config), GSETTINGS_BACKEND="memory",
    )
    downloads = config.parent / "Documents and Downloads" / "Загрузки"
    downloads.mkdir(parents=True)
    (config / "user-dirs.dirs").write_text(f'XDG_DOWNLOAD_DIR="{downloads}"\n')
    desktop = data / "applications"
    desktop.mkdir()
    for identifier, name in (("other.desktop", "Other Browser"), ("quickie.desktop", "Quickie")):
        (desktop / identifier).write_text(
            f"[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/false %u\n"
            "MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n"
        )
    (config / "mimeapps.list").write_text(
        "[Default Applications]\n" + "".join(f"{scheme}=other.desktop;\n" for scheme in SCHEMES)
    )
    return downloads


def default_is(story, identifier):
    return all(
        check_output(["gio", "mime", scheme], env=story.environment, text=True)
        .splitlines()[0].endswith(f": {identifier}")
        for scheme in SCHEMES
    )


def named_window(story, title):
    def found():
        for window in story.clients():
            if story.window_title(window) == title:
                return window
        return None

    window = story.wait(found, f"the native {title} window")
    story.focus(window)
    story.move_pointer(window, 4, 4)
    return window


def general(story, browser):
    story.focus(browser)
    story.key("ctrl+comma")
    return named_window(story, "Quickie Settings")


def viewport(story, state, previous=None, expected=None):
    def reported():
        prefix = f"/{FIXTURE}?{state}="
        with story.server.request_lock:
            reports = dict(story.server.requests)
        for path, count in reversed(reports.items()):
            if path.startswith(prefix) and (previous is None or count > previous.get(path, 0)):
                width, height = map(int, path.removeprefix(prefix).split("x"))
                if expected is None or (width, height) == expected:
                    return width, height
        return None

    return story.wait(reported, f"the live {state} viewport to report {expected or 'its size'}")


def requests(story):
    with story.server.request_lock:
        return story.server.requests.copy()


def edge(story, window, inset):
    def painted():
        image = story.picture(window)
        y = image.height // 2
        outside = image.getpixel((6 * story.scale, y)) != PAGE
        inside = image.getpixel((14 * story.scale, y)) == PAGE
        return image if inside and outside == inset else None

    return story.wait(painted, f"the live page to paint {'inside' if inset else 'up to'} its edge")


def button(story, window, y):
    image = story.picture(window)
    story.click(window, image.width - 60 * story.scale, y * story.scale)
    story.move_pointer(window, 4, 4)


def error_visible(story, window, y, failure):
    box = tuple(value * story.scale for value in (32, y, 410, y + 16))
    blank = story.picture(window).crop(box)

    def changed():
        row = story.picture(window).crop(box)
        return ImageChops.difference(blank, row).getbbox()

    return lambda: story.wait(changed, failure)


def painted_general(story, window):
    title = tuple(value * story.scale for value in (24, 22, 145, 44))

    def rendered():
        image = story.picture(window)
        contrast = image.crop(title).convert("L").getextrema()
        return image if contrast[1] - contrast[0] > 100 else None

    return story.wait(rendered, "the General heading to paint")


def settled_general(story, window, start, inset=False, default=False):
    def region(box):
        return tuple(value * story.scale for value in box)

    unchanged = [
        region((32, y, 410, y + 15)) for y in (148, 241, 351)
    ]
    browser = region((60, 104, 390, 128))
    switch = region((516, 300, 558, 320))

    def differs(image, box):
        return ImageChops.difference(start.crop(box), image.crop(box)).getbbox() is not None

    def rendered():
        image = story.picture(window)
        if any(differs(image, box) for box in unchanged):
            return None
        return image if differs(image, switch) == inset and differs(image, browser) == default else None

    return story.wait(rendered, "General's visible saved preferences")


def unavailable_downloads(story, window, downloads):
    downloads.rmdir()
    try:
        status = error_visible(story, window, 241, "the missing Downloads folder message")
        button(story, window, DOWNLOADS_Y)
        status()
    finally:
        downloads.mkdir()


def denied_default(story, window):
    config = Path(story.environment["XDG_CONFIG_HOME"])
    config.chmod(0o500)
    try:
        status = error_visible(story, window, 148, "the denied default handler message")
        button(story, window, DEFAULT_Y)
        status()
        assert default_is(story, "other.desktop")
    finally:
        config.chmod(0o700)


def denied_appearance(story, browser, window):
    profile = Path(story.environment["XDG_DATA_HOME"]) / "quickie" / "shell.sqlite"
    with sqlite3.connect(profile) as database:
        database.execute(
            "CREATE TRIGGER refuse_appearance BEFORE INSERT ON appearance "
            "BEGIN SELECT RAISE(ABORT, 'test refused the appearance change'); END"
        )
    try:
        status = error_visible(story, window, 351, "the failed appearance save message")
        button(story, window, INSET_Y)
        log = story.output / "browser.log"
        story.wait(
            lambda: "test refused the appearance change" in log.read_text(),
            "the SQLite denial to reach Quickie",
        )
        status()
        edge(story, browser, False)
        with sqlite3.connect(profile) as database:
            assert database.execute("SELECT inset_page FROM appearance").fetchone() is None
    finally:
        with sqlite3.connect(profile) as database:
            database.execute("DROP TRIGGER refuse_appearance")


def run(story):
    # Arrange
    downloads = configure_desktop(story)
    assert default_is(story, "other.desktop")
    browser = story.open(FIXTURE)
    edge(story, browser, False)
    full_width, full_height = viewport(story, "windowed")
    window = general(story, browser)
    start = story.save(painted_general(story, window), "general")

    denied_default(story, window)
    window = general(story, browser)
    settled_general(story, window, start)
    unavailable_downloads(story, window, downloads)
    window = general(story, browser)
    settled_general(story, window, start)
    denied_appearance(story, browser, window)
    window = general(story, browser)
    settled_general(story, window, start)

    # Act and assert the preference while its native switch remains visible.
    before = requests(story)
    button(story, window, INSET_Y)
    inset = (full_width - 16, full_height - 16)
    viewport(story, "windowed", before, inset)
    story.save(settled_general(story, window, start, inset=True), "general-inset")

    # Act and assert the opt-in through the real GIO associations.
    button(story, window, DEFAULT_Y)
    story.wait(lambda: default_is(story, "quickie.desktop"), "both web handlers to become Quickie")
    story.save(settled_general(story, window, start, inset=True, default=True), "general-default")

    story.key("Escape")
    story.wait(lambda: window not in story.clients(), "Escape to close General")
    image = edge(story, browser, True)

    # Exercise WebKit document fullscreen and the inset that returns after exit.
    before = requests(story)
    story.click(browser, image.width // 2, image.height // 2)
    viewport(story, "fullscreen", before, (full_width, full_height))
    edge(story, browser, False)
    before = requests(story)
    story.key("Escape")
    viewport(story, "windowed", before, inset)
    edge(story, browser, True)

    story.close(browser)
    before = requests(story)
    browser = story.open()
    assert default_is(story, "quickie.desktop")
    viewport(story, "windowed", before, inset)
    edge(story, browser, True)
    window = general(story, browser)
    before = requests(story)
    button(story, window, INSET_Y)
    viewport(story, "windowed", before, (full_width, full_height))
    settled_general(story, window, start, default=True)
    button(story, window, UPDATES_Y)
    updates = named_window(story, "Software Updates")
    story.key("Escape")
    story.wait(
        lambda: window not in story.clients() and updates not in story.clients(),
        "the settings and updates windows to close",
    )
    edge(story, browser, False)
    story.close(browser)
