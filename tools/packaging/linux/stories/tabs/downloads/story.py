"""Capture the panel and check native transfers from a real loopback server."""

from pathlib import Path
import sqlite3
import time

from PIL import ImageChops

from ..keyboard import control_tab
from ..story import address_visible, text_updated
from .endpoint import PAYLOAD, SHUTDOWN_PAYLOAD, endpoint

PAGE = (28, 57, 86)
PRIVATE_PAGE = (39, 56, 42)
LINK = (49, 120, 119)


def count(server, path):
    with server.count_lock:
        return server.counts[path]


def painted(story, window, color):
    def match():
        image = story.picture(window)
        return image if image.getpixel((15 * story.scale, image.height // 2)) == color else None

    return story.wait(match, f"the downloads fixture to paint {color}")


def fixture(story, window, url, color=PAGE):
    story.key("ctrl+l")
    address = address_visible(story, window)
    story.paste(url)
    text_updated(story, window, address, 20)
    story.key("Return")

    return painted(story, window, color)


def click_link(story, window, image, row):
    x = 280 * story.scale
    matches = [y for y in range(image.height) if image.getpixel((x, y)) == LINK]
    runs = [y for i, y in enumerate(matches) if i == 0 or y != matches[i - 1] + 1]
    assert len(runs) == 6, f"expected all six real links, got {runs}"
    story.click(window, x, runs[row] + 15 * story.scale)
    story.move_pointer(window, 4, 4)


def panel(story, window, base):
    def changed():
        image = story.picture(window)
        width = image.width
        left = width - 470 * story.scale
        box = (left, 0, width, image.height)
        diff = ImageChops.difference(base.crop(box), image.crop(box)).getbbox()
        if diff and diff[2] - diff[0] > 300 * story.scale:
            return image, (diff[0] + left, diff[1], diff[2] + left, diff[3])
        return None

    return story.wait(changed, "the native Downloads panel")


def close_panel(story, window, base):
    story.key("Escape")
    edge = (base.width - 470 * story.scale, 0, base.width, base.height)
    story.wait(
        lambda: ImageChops.difference(story.picture(window).crop(edge), base.crop(edge)).getbbox()
        is None,
        "Escape to dismiss Downloads and expose the page",
    )


def fetch(story, server, window, image, row, path):
    click_link(story, window, image, row)
    story.wait(lambda: count(server, path) == 1, f"a user click to request {path}")


def file_contains(story, path, data):
    story.wait(lambda: path.is_file() and path.read_bytes() == data, f"real payload at {path.name}")


def partial_file(folder, previous):
    for file in folder.glob(".quickie-download-*/payload.wkdownload"):
        try:
            if file not in previous and file.stat().st_size > 0:
                return file
        except FileNotFoundError:
            continue
    return None


def partial(story, folder, previous=()):
    return story.wait(
        lambda: partial_file(folder, previous),
        "WebKit to write bytes to its private staging directory",
    )


def mixed(story, server, window, idle, folder):
    fetch(story, server, window, idle, 2, "/retry")
    interrupted = partial(story, folder)
    server.release["retry"].set()
    story.wait(lambda: not interrupted.parent.exists(), "the failed transfer to discard its stage")

    fetch(story, server, window, idle, 3, "/cancel")
    cancelled = partial(story, folder)
    story.key("ctrl+j")
    _, bounds = panel(story, window, idle)
    _, top, right, _ = bounds
    story.click(window, right - 65 * story.scale, top + 117 * story.scale)
    story.move_pointer(window, 4, 4)
    story.wait(lambda: not cancelled.parent.exists(), "Cancel to discard the partial file")
    server.release["cancel"].set()
    assert not (folder / "dataset.zip").exists()
    close_panel(story, window, idle)

    fetch(story, server, window, idle, 4, "/known")
    known = partial(story, folder)
    fetch(story, server, window, idle, 5, "/unknown")
    partial(story, folder, (known,))

    story.key("ctrl+j")
    image, (left, top, right, _) = panel(story, window, idle)
    story.save(image, "downloads-mixed")
    for _ in range(3):
        story.key("Down")
    first_row = (left, top + 85 * story.scale, right, top + 105 * story.scale)

    def moved():
        current = story.picture(window)
        return current if ImageChops.difference(current.crop(first_row), image.crop(first_row)).getbbox() else None

    story.save(story.wait(moved, "a different selected row"), "downloads-keys")
    story.key("Return")
    story.wait(lambda: count(server, "/retry") == 2, "Enter to retry the selected failed download")
    file_contains(story, folder / "quarterly report 2.txt", PAYLOAD)
    close_panel(story, window, idle)

    racer = folder / "archive.bin"
    racer.write_bytes(b"created while the browser was downloading")
    server.release["known"].set()
    story.wait(lambda: not known.parent.exists(), "publication to discard the stage after a collision")
    assert racer.read_bytes() == b"created while the browser was downloading"


def throwaway(story, server, window, url, idle, folder):
    story.key("ctrl+shift+n")
    private = fixture(story, window, f"{url}?throwaway", PRIVATE_PAGE)
    story.wait(lambda: count(server, "/automatic") == 2, "the fixture in the throwaway tab")
    existing = list(folder.glob(".quickie-download-*/payload.wkdownload"))
    fetch(story, server, window, private, 0, "/private")
    staged = partial(story, folder, existing)
    server.release["private"].set()
    story.wait(lambda: not staged.parent.exists(), "the interrupted throwaway file to leave staging")
    with control_tab(story, hold_ms=325):
        pass
    painted(story, window, PAGE)
    story.key("ctrl+j")
    image, _ = panel(story, window, idle)
    story.save(image, "downloads-throwaway")
    story.key("Return")
    story.wait(lambda: count(server, "/private") == 2, "retry from the original throwaway tab")
    file_contains(story, folder / "throwaway-only.txt", PAYLOAD)
    close_panel(story, window, idle)
    with control_tab(story, hold_ms=325):
        pass
    painted(story, window, PRIVATE_PAGE)
    story.key("ctrl+w")
    story.key("ctrl+j")
    panel(story, window, idle)
    assert not (folder / "network-proof 4.txt").exists()
    assert not (folder / "automatic.txt").exists()


def saved_downloads(story, *, by_url=False):
    database = Path(story.environment["XDG_DATA_HOME"]) / "quickie" / "shell.sqlite"
    identity = "url" if by_url else "file_name"
    with sqlite3.connect(f"{database.as_uri()}?mode=ro", uri=True) as db:
        return dict(db.execute(f"SELECT {identity}, state FROM downloads"))


def wait_for_file_event(probe, message):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if probe():
            return
        time.sleep(0.001)
    raise RuntimeError(f"Timed out waiting for {message}")


def close_at_publication(story, server, window, idle, folder):
    existing = set(folder.glob(".quickie-download-*/payload.wkdownload"))
    click_link(story, window, idle, 5)
    wait_for_file_event(lambda: partial_file(folder, existing), "the shutdown transfer to stage")
    server.release["shutdown"].set()
    # Close at the disk commit, ahead of the panel's next periodic refresh.
    published = folder / "shutdown-completed.txt"
    wait_for_file_event(published.is_file, "the completed file to appear")
    story.close(window)
    assert published.read_bytes() == SHUTDOWN_PAYLOAD


def restored(story, server, folder):
    saved = saved_downloads(story)
    assert saved.get("network-proof 3.txt") == "finished", saved
    assert saved.get("quarterly report.txt") == "finished", saved
    assert saved.get("shutdown-completed.txt") == "finished", saved
    assert "throwaway-only.txt" not in saved, saved

    window = story.open()

    idle = painted(story, window, PAGE)
    story.key("ctrl+j")
    image, (_left, top, right, bottom) = panel(story, window, idle)
    assert bottom - top > 280 * story.scale, "the download history did not restore into the panel"
    story.save(image, "downloads-restored")
    story.click(window, right - 195 * story.scale, top + 57 * story.scale)
    story.close(window)
    assert not saved_downloads(story), "Clear Finished did not clear restored records"
    assert (folder / "network-proof 3.txt").read_bytes() == PAYLOAD
    assert list(folder.glob("shutdown-completed*.txt")) == [folder / "shutdown-completed.txt"]
    assert (folder / "shutdown-completed.txt").read_bytes() == SHUTDOWN_PAYLOAD
    assert count(server, "/unknown") == 2, "reopening requested a duplicate file"
    if story.scale == 1:
        assert (folder / "throwaway-only.txt").read_bytes() == PAYLOAD


def run(story):
    config = Path(story.environment["XDG_CONFIG_HOME"])
    folder = config.parent / "Downloads"
    folder.mkdir()
    (config / "user-dirs.dirs").write_text(f'XDG_DOWNLOAD_DIR="{folder}"\n')
    sentinel = folder / "network-proof.txt"
    sentinel.write_bytes(b"already here")
    broken = folder / "network-proof 2.txt"
    broken.symlink_to(folder / "missing-target")
    with endpoint() as server:
        window = story.open()
        url = f"http://127.0.0.1:{server.server_port}/"
        idle = fixture(story, window, url)
        story.wait(lambda: count(server, "/automatic") == 1, "the unsolicited attachment request")
        story.key("ctrl+j")
        image, _ = panel(story, window, idle)
        story.save(image, "downloads-empty")
        close_panel(story, window, idle)

        fetch(story, server, window, idle, 0, "/saved")
        file_contains(story, folder / "network-proof 3.txt", PAYLOAD)
        assert sentinel.read_bytes() == b"already here"
        assert broken.is_symlink() and not broken.exists()
        assert not (folder / "automatic.txt").exists()
        fetch(story, server, window, idle, 1, "/named")
        file_contains(story, folder / "quarterly report.txt", PAYLOAD)
        mixed(story, server, window, idle, folder)

        if story.scale == 1:
            throwaway(story, server, window, url, idle, folder)
            close_panel(story, window, idle)
        close_at_publication(story, server, window, idle, folder)
        assert not list(folder.glob(".quickie-download-*")), "closing left staged files behind"
        restored(story, server, folder)
