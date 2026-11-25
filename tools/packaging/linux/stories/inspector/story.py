"""Exercise the docked and detached GTK Web Inspector in a fresh profile."""

from PIL import ImageChops

from .accessibility import detach, dock_control

FIXTURE = "inspector/fixture.html"
MARKER = (95, 226, 176)
DOCK_INSET = 12
DEFAULT_DOCK_HEIGHT = 300


def viewport_height(image, scale):
    """Recover the page height from the fixed marker at its bottom-right corner."""
    x = image.width - 18 * scale
    for y in reversed(range(image.height)):
        if image.getpixel((x, y)) == MARKER:
            return y + 1 + 10 * scale
    return 0


def has_frontend(image, top, scale):
    """Require actual toolbar and content pixels, beyond a flat dock placeholder."""
    toolbar = image.crop((0, top, image.width, top + 28 * scale))
    content = image.crop((0, top + 40 * scale, image.width, image.height))
    return toolbar.getcolors(16) is None and content.getcolors(16) is None


def docked_bounds(image, scale, floating):
    if floating:
        inset = DOCK_INSET * scale
        bottom = image.height - inset
        bounds = (inset, bottom - DEFAULT_DOCK_HEIGHT * scale, image.width - inset, bottom)
    else:
        top = viewport_height(image, scale)
        if not 0 < top <= image.height - 150 * scale:
            return None
        bounds = (0, top, image.width, image.height)
    if has_frontend(image.crop(bounds), 0, scale):
        return bounds
    return None


def click(story, window, x, y):
    story.click(window, x, y)
    story.move_pointer(window, 4, 4)


def prompt_row(image, scale):
    # Inspector versions can reserve a row for the evaluation preview below the prompt,
    # and detached windows can add a left frame inset before the chevron.
    # Find the bottommost prompt chevron so the click and caret check follow the editor.
    for y in range(image.height - 5 * scale, image.height - 52 * scale, -1):
        for x in range(5 * scale, 32 * scale):
            red, green, blue = image.getpixel((x, y))[:3]
            if blue - red > 60 and blue - green > 25:
                return y - 4 * scale
    return None


def prove_console(story, window, mode, bounds, report):
    scale = story.scale
    image = story.picture(window)
    left, top, right, bottom = bounds or (0, 0, image.width, image.height)
    if mode == "docked":
        content = (left, top + 40 * scale, right, bottom - 36 * scale)
        elements = image.crop(content)

        def console_visible():
            console = story.picture(window).crop(content)
            changes = ImageChops.difference(elements, console).getbbox()
            return changes and changes[3] - changes[1] >= 60 * scale

        # Seat native focus in the dock and use the frontend's Console shortcut.
        click(story, window, (left + right) // 2, top + 75 * scale)
        story.key("ctrl+alt+c")
        story.wait(console_visible, "docked Inspector Console tab")

    row = top + story.wait(
        lambda: prompt_row(story.picture(window).crop((left, top, right, bottom)), scale),
        f"{mode} Console prompt",
    )

    click(story, window, left + 32 * scale, row)
    story.key("ctrl+grave")
    prompt = (left, row - 9 * scale, (left + right) // 2, row + 9 * scale)
    empty = story.picture(window).crop(prompt)
    request = f"{FIXTURE}?inspector={report}"
    prior = story.requests(request)
    page_input = f"{FIXTURE}?page=typed"
    page_input_before = story.requests(page_input)
    story.key(*mode, "parenleft", "parenright")

    def command_ready():
        changes = ImageChops.difference(empty, story.picture(window).crop(prompt)).getbbox()
        return changes and changes[2] - changes[0] >= 40 * scale

    story.wait(command_ready, f"{mode} command typed into the Console prompt")
    story.key("Return")
    story.wait(lambda: story.requests(request) > prior, f"{mode} Inspector Console request")
    assert story.requests(page_input) == page_input_before, "Inspector keys reached the inspected page"
    story.move_pointer(window, 4, 4)


def painted(story, window, condition):
    image = story.picture(window)
    return image if condition(image) else None


def run(story):
    story.environment.update(GTK_A11Y="atspi", LC_ALL="C.UTF-8")
    browser = story.open(FIXTURE)
    scale = story.scale

    def is_full_page():
        return painted(story, browser, lambda image: viewport_height(image, scale) == image.height)

    initial = story.wait(is_full_page, "inspector fixture painted")
    original_clients = story.clients()
    story.key("F12")
    control, floating = dock_control(story)
    bounds = story.wait(
        lambda: docked_bounds(story.picture(browser), scale, floating),
        "docked Inspector frontend painted",
    )
    inspected_height = initial.height if floating else bounds[1]
    viewport = f"{initial.width // scale}x{inspected_height // scale}"
    prove_console(story, browser, "docked", bounds, f"docked&viewport={viewport}")
    docked_frame = story.wait(
        lambda: painted(story, browser, lambda image: docked_bounds(image, scale, floating)),
        "docked Inspector after its Console request",
    )
    story.save(docked_frame, "inspector-docked")

    if floating:
        request = f"{FIXTURE}?page=clicked"
        before = story.requests(request)
        click(story, browser, bounds[0] // 2, (bounds[1] + bounds[3]) // 2)
        story.wait(lambda: story.requests(request) > before, "the gutter click to reach the page")

    detach(story, control, floating)
    detached = story.wait(
        lambda: next(iter(story.clients() - original_clients), None),
        "detached Inspector window",
    )
    story.focus(browser)
    story.wait(is_full_page, "inspected page restored to full height")
    story.focus(detached)
    story.wait(
        lambda: has_frontend(story.picture(detached), 50 * scale, scale),
        "detached Inspector frontend painted",
    )
    prove_console(story, detached, "detached", None, "detached")
    detached_frame = story.wait(
        lambda: painted(story, detached, lambda image: has_frontend(image, 50 * scale, scale)),
        "detached Inspector after its Console request",
    )
    story.save(detached_frame, "inspector-detached")

    story.key("F12")
    story.wait(lambda: detached not in story.clients(), "F12 closes detached Inspector")
