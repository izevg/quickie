"""Exercise picture presentation with a real native browser surface."""

import time

from PIL import Image, ImageChops

from ..keyboard import control_tab

FIXTURE = "tabs/media/fixture.html"
VIDEO = "tabs/media/video.html"
WINDOWED = (49, 84, 117)
FULLSCREEN = (35, 118, 80)
VIDEO_TAB = (48, 60, 121)
OTHER_TAB = (110, 71, 84)
PLAYER = (32, 33, 37)


def painted(story, window, color):
    def matches():
        image = story.picture(window)
        point = (32 * story.scale, image.height // 2)
        return image if image.getpixel(point) == color else None

    return story.wait(matches, f"the page to paint {color} on the original surface")


def fullscreen(story):
    window = story.open(FIXTURE)
    idle = painted(story, window, WINDOWED)
    clients = story.clients()
    for attempt in (1, 2):
        story.click(window, idle.width // 2, idle.height // 2)
        story.wait(
            lambda: story.requests(f"{FIXTURE}?state=entered") == attempt,
            "document.fullscreenElement to identify the requested element",
        )
        shown = painted(story, window, FULLSCREEN)
        assert shown.size == idle.size, "WebKit resized the desktop toplevel"
        assert story.clients() == clients, "WebKit opened another native surface"
        story.save(shown, f"fullscreen-entered-{attempt}")

        story.key("Escape")
        story.wait(
            lambda: story.requests(f"{FIXTURE}?state=left") == attempt,
            "Escape to clear document.fullscreenElement",
        )
        restored = painted(story, window, WINDOWED)
        assert restored.size == idle.size
        assert story.clients() == clients
        story.save(restored, f"fullscreen-restored-{attempt}")
    story.close(window)


def reports(story, state):
    return story.requests(f"{VIDEO}?state={state}")


def player_bounds(story, image):
    top = 40 * story.scale
    page = image.crop((0, top, image.width, image.height))
    surface = Image.new("RGB", page.size, PLAYER)
    difference = ImageChops.difference(page, surface)
    matches = ImageChops.invert(difference.convert("L")).point(
        lambda value: 255 if value == 255 else 0
    )
    bounds = matches.getbbox()
    if bounds and bounds[2] - bounds[0] >= 270 * story.scale:
        x1, y1, x2, y2 = bounds
        return (x1, y1 + top, x2, y2 + top)
    return None


def player_in(story, window, position):
    expected = tuple(position.split("-"))

    def matching():
        image = story.picture(window)
        bounds = player_bounds(story, image)
        if bounds:
            left, top, right, bottom = bounds
            vertical = "top" if top + bottom < image.height else "bottom"
            horizontal = "left" if left + right < image.width else "right"
            if (vertical, horizontal) == expected:
                return image, bounds
        return None

    return story.wait(matching, f"the native mini-player controls at the {position} corner")


def click_control(story, window, bounds, index):
    left, top, right, bottom = bounds
    story.click(window, left + (2 * index + 1) * (right - left) // 10, (top + bottom) // 2)
    story.move_pointer(window, 4, 4)


def hidden(story, window):
    def matching():
        image = story.picture(window)
        return image if not player_bounds(story, image) else None

    return story.wait(matching, "the mini-player controls to leave the surface")


def switch(story):
    with control_tab(story, hold_ms=325):
        pass


def still_advancing(story):
    count = reports(story, "advanced")
    story.wait(lambda: reports(story, "advanced") >= count + 3, "the video clock to advance")


def paused(story):
    count = reports(story, "advanced")
    deadline = time.monotonic() + 0.75
    story.wait(lambda: time.monotonic() >= deadline, "a paused playback interval")
    assert reports(story, "advanced") == count, "playback advanced after a native pause"


def floating_pixels(story, window, image, bounds):
    x1, y1, x2, _ = bounds
    region = (x1 + 12 * story.scale, y1 - 160 * story.scale, x2 - 12 * story.scale,
              y1 - 32 * story.scale)
    first = image.crop(region)
    colors = first.resize((32, 24)).getcolors(769)
    assert len(colors) > 30, "the mini-player does not show the decoded clip"

    def changed():
        next_image = story.picture(window)
        difference = ImageChops.difference(first, next_image.crop(region))
        return difference.getbbox()

    story.wait(changed, "new video pixels to paint in the same mini-player")


def playback(story):
    window = story.open(VIDEO)
    clients = story.clients()
    painted(story, window, VIDEO_TAB)
    story.click(window, 82 * story.scale, 110 * story.scale)
    story.wait(lambda: story.requests(f"{VIDEO}?other") == 1, "the other tab to navigate")
    painted(story, window, OTHER_TAB)
    switch(story)
    source = painted(story, window, VIDEO_TAB)
    story.click(window, source.width // 2, 108 * story.scale)
    story.wait(lambda: reports(story, "playing") == 1, "real playback to begin")
    story.wait(lambda: reports(story, "audio") > 0, "the video to expose its Opus audio track")
    still_advancing(story)

    switch(story)
    painted(story, window, OTHER_TAB)
    image, bottom = player_in(story, window, "bottom-right")
    assert story.clients() == clients, "the in-window mini player used another native toplevel"
    floating_pixels(story, window, image, bottom)
    story.save(image, "mini-player-bottom")

    click_control(story, window, bottom, 0)
    story.wait(lambda: reports(story, "paused") == 1, "the native Pause button to pause the clip")
    paused(story)
    story.save(story.picture(window), "mini-player-paused")
    click_control(story, window, bottom, 0)
    story.wait(lambda: reports(story, "playing") == 2, "the native Play button to resume the clip")
    still_advancing(story)

    click_control(story, window, bottom, 2)
    image, top = player_in(story, window, "top-right")
    story.save(image, "mini-player-top")
    for position in ("top-left", "bottom-left"):
        click_control(story, window, top, 2)
        image, top = player_in(story, window, position)
        floating_pixels(story, window, image, top)
        still_advancing(story)
        story.save(image, f"mini-player-{position}")
    assert reports(story, "playing") == 2 and reports(story, "paused") == 1
    assert story.requests(VIDEO) == 1, "Moving the player reloaded the source document"

    click_control(story, window, top, 2)
    _, bottom = player_in(story, window, "bottom-right")
    click_control(story, window, bottom, 3)
    story.save(hidden(story, window), "mini-player-minimized")
    still_advancing(story)
    assert reports(story, "paused") == 1, "Minimize paused the source"

    switch(story)
    painted(story, window, VIDEO_TAB)
    switch(story)
    _, bottom = player_in(story, window, "bottom-right")
    click_control(story, window, bottom, 1)
    story.save(painted(story, window, VIDEO_TAB), "mini-player-back-to-tab")
    hidden(story, window)
    still_advancing(story)
    assert story.requests(VIDEO) == 1, "Back to tab reloaded the source document"

    switch(story)
    _, bottom = player_in(story, window, "bottom-right")
    click_control(story, window, bottom, 4)
    hidden(story, window)
    story.wait(lambda: reports(story, "paused") == 2, "Close to pause the original video")
    paused(story)
    switch(story)
    painted(story, window, VIDEO_TAB)
    assert story.clients() == clients
    assert story.requests(VIDEO) == 1, "The playhead was left in a different document"
    story.close(window)


def run(story):
    fullscreen(story)
    playback(story)
