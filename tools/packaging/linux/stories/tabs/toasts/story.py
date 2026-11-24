"""Exercise GTK notifications from real browser commands and a local transfer."""

import json
from pathlib import Path
import time

from PIL import Image, ImageChops

from tabs.downloads.endpoint import PAYLOAD, endpoint
from tabs.downloads.story import file_contains, partial, saved_downloads
from tabs.story import address_visible, text_updated

FIXTURE = "tabs/toasts/fixture.html"
PAGE = (38, 61, 87)
FULLSCREEN = (39, 98, 71)
ZOOM_PROBE = (102, 202, 167)


def painted(story, window, color, x=14, y=90):
    def match():
        image = story.picture(window)
        return image if image.getpixel((x * story.scale, y * story.scale)) == color else None

    return story.wait(match, f"the page to paint {color}")


def rows(story, image, page=PAGE):
    scale = story.scale
    center = image.width // 2
    left = max(0, center - 275 * scale)
    top = image.height - 180 * scale
    area = image.crop((left, top, image.width - left, image.height - 12 * scale))
    diff = ImageChops.difference(area, Image.new("RGB", area.size, page))
    red, green, blue = diff.split()
    mask = ImageChops.lighter(ImageChops.lighter(red, green), blue).point(
        lambda value: 255 if value >= 27 else 0
    )
    bands = []
    start = None
    for y in range(mask.height):
        row = mask.crop((0, y, mask.width, y + 1)).getbbox()
        draws_pill = row and row[2] - row[0] >= 30 * scale
        if draws_pill and start is None:
            start = y
        if not draws_pill and start is not None:
            if y - start >= 11 * scale:
                bounds = mask.crop((0, start, mask.width, y)).getbbox()
                bands.append((left + bounds[0], top + start, left + bounds[2], top + y))
            start = None
    return bands


def visible(story, window, count, hue=None):
    def match():
        image = story.picture(window)
        shown = rows(story, image)
        if len(shown) == count and (hue is None or status_icon(story, image, shown[-1], hue)):
            return image, shown
        return None

    description = f"{count} centered native toast pills"
    if hue:
        description += f", the most recent with a {hue} status icon"
    return story.wait(match, description)


def status_icon(story, image, bounds, hue):
    left, top, _, bottom = bounds
    area = image.crop((left, top, left + 40 * story.scale, bottom))
    if hue == "green":
        colored = sum(g > r + 25 and g > b + 6 for r, g, b in area.getdata())
    else:
        colored = sum(r > g + 30 and r > b + 22 for r, g, b in area.getdata())
    return colored >= 4 * story.scale ** 2


def one(story, window):
    idle = painted(story, window, PAGE)
    story.key("shift+alt+f")
    target = tuple(value * story.scale for value in (51, 132, 77, 158))
    story.wait(
        lambda: ImageChops.difference(idle.crop(target), story.picture(window).crop(target)).getbbox(),
        "the first background Link Hint",
    )
    started = time.monotonic()
    story.key("a")
    story.wait(lambda: story.requests(f"{FIXTURE}?destination=background") == 1, "background link load")
    image, [(left, top, right, bottom)] = visible(story, window, 1)
    assert abs(left + right - image.width) <= 5 * story.scale
    assert abs(image.height - bottom - 24 * story.scale) <= 5 * story.scale
    story.save(image, "toast-one")

    story.click(window, (left + right) // 2, (top + bottom) // 2)
    story.move_pointer(window, 4, 4)
    story.wait(lambda: story.requests(f"{FIXTURE}?click=page") == 1, "the page hit beneath the toast")
    visible(story, window, 0)
    elapsed = time.monotonic() - started
    assert 2.8 <= elapsed <= 4.5, f"expected a three-second toast hold, got {elapsed:.2f} s"


def stacked(story, window):
    story.key("ctrl+shift+c")
    _, [(left, _, right, _)] = visible(story, window, 1, "green")
    copy_width = right - left
    story.key("ctrl+alt+4")
    _, [oldest, recent] = visible(story, window, 2)
    assert oldest[2] - oldest[0] > recent[2] - recent[0], "the most recent toast was not lowest"
    story.key("ctrl+alt+5", "ctrl+alt+6")

    def newest_three():
        image = story.picture(window)
        shown = rows(story, image)
        widths = [right - left for left, _, right, _ in shown]
        if len(widths) == 3 and max(widths) < copy_width and widths[-1] + 16 * story.scale < min(widths[:2]):
            return image
        return None

    story.save(story.wait(newest_three, "the oldest toast gives way to the three recent pins"), "toast-stacked")
    visible(story, window, 0)


def tagged(story, window):
    story.key("ctrl+equal")
    visible(story, window, 1)
    story.key("ctrl+equal")
    painted(story, window, ZOOM_PROBE, 95, 130)
    image, _ = visible(story, window, 1)
    story.save(image, "toast-tagged")

    story.key("ctrl+shift+c")
    visible(story, window, 2)
    story.key("ctrl+0")
    painted(story, window, PAGE, 95, 130)
    _, [older, newest] = visible(story, window, 2)
    assert older[2] - older[0] < newest[2] - newest[0], "tagged replacement moved to a new slot"
    visible(story, window, 0)


def click(story, window, x, y):
    story.click(window, x * story.scale, y * story.scale)
    story.move_pointer(window, 4, 4)


def downloads(story, window, folder, server):
    story.key("ctrl+shift+c")
    visible(story, window, 1)
    click(story, window, 304, 154)
    file_contains(story, folder / "network-proof.txt", PAYLOAD)
    image, _ = visible(story, window, 3, "green")
    story.save(image, "toast-kinds")
    visible(story, window, 0)

    click(story, window, 505, 154)
    partial(story, folder)
    server.release["retry"].set()
    story.wait(
        lambda: saved_downloads(story).get("quarterly report.txt") == "failed",
        "the interrupted transfer to reach the failed state",
    )
    image, _ = visible(story, window, 2, "red")
    story.save(image, "toast-problem")
    visible(story, window, 0)


def fullscreen(story, window):
    story.key("ctrl+shift+c")
    visible(story, window, 1)
    click(story, window, 820, 154)
    story.wait(lambda: story.requests(f"{FIXTURE}?state=fullscreen") == 1, "page fullscreen entry")
    image = painted(story, window, FULLSCREEN)
    assert not rows(story, image, FULLSCREEN), "entering fullscreen kept the active toast"
    story.key("ctrl+shift+c")
    deadline = time.monotonic() + 0.25
    story.wait(lambda: time.monotonic() >= deadline, "the fullscreen command to reach GTK")
    assert not rows(story, story.picture(window), FULLSCREEN), "a toast opened in fullscreen"
    story.key("Escape")
    painted(story, window, PAGE)
    visible(story, window, 0)


def persistent_zoom(story, window, path):
    story.key("ctrl+equal", "ctrl+equal")
    painted(story, window, ZOOM_PROBE, 95, 130)
    story.close(window)

    window = story.open(path)
    painted(story, window, ZOOM_PROBE, 95, 130)
    prior = story.requests(path)
    story.key("ctrl+shift+n")
    address = address_visible(story, window)
    story.paste(story.url(path))
    text_updated(story, window, address, 20)
    story.key("Return")
    story.wait(lambda: story.requests(path) > prior, "the throwaway tab loads the same page")
    painted(story, window, PAGE, 95, 130)
    story.key("ctrl+w")
    painted(story, window, ZOOM_PROBE, 95, 130)
    story.key("ctrl+0")
    painted(story, window, PAGE, 95, 130)
    story.close(window)


def run(story):
    config = Path(story.environment["XDG_CONFIG_HOME"])
    folder = config.parent / "Downloads"
    folder.mkdir()
    (config / "user-dirs.dirs").write_text(f'XDG_DOWNLOAD_DIR="{folder}"\n')
    keys = config / "quickie"
    keys.mkdir()
    (keys / "keybindings.json").write_text(
        json.dumps([{"key": "ctrl+6", "command": "-quickie.tab.goToSlot6"}]),
    )
    with endpoint() as server:
        path = f"{FIXTURE}?downloads={server.server_port}"
        window = story.open(path)
        one(story, window)
        stacked(story, window)
        tagged(story, window)
        downloads(story, window, folder, server)
        fullscreen(story, window)
        persistent_zoom(story, window, path)
