"""Exercise the docked and detached GTK Web Inspector in a fresh profile."""

from PIL import ImageChops

FIXTURE = "inspector/fixture.html"
MARKER = (95, 226, 176)


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


def docked_top(image, scale):
    top = viewport_height(image, scale)
    if 0 < top <= image.height - 150 * scale and has_frontend(image, top, scale):
        return top
    return 0


def click(story, window, x, y):
    story.click(window, x, y)
    story.move_pointer(window, 4, 4)


def prompt_focused(image, scale):
    # CodeMirror paints a contrasting vertical caret beside the empty prompt.
    left, top = 15 * scale, image.height - 23 * scale
    strip = image.crop((left, top, 40 * scale, image.height - 4 * scale)).convert("L")
    offset = 3 * scale
    return any(
        sum(abs(strip.getpixel((x, y)) - strip.getpixel((x + offset, y))) > 65
            for y in range(strip.height)) >= 8 * scale
        for x in range(strip.width - offset)
    )


def prove_console(story, window, mode, top):
    scale = story.scale
    image = story.picture(window)
    if mode == "docked":
        content = (0, top + 40 * scale, image.width, image.height - 36 * scale)
        elements = image.crop(content)

        def console_visible():
            console = story.picture(window).crop(content)
            changes = ImageChops.difference(elements, console).getbbox()
            return changes and changes[3] - changes[1] >= 60 * scale

        # The fresh Inspector opens on Elements. Undocking preserves the active Console tab.
        click(story, window, 290 * scale, top + 14 * scale)
        story.wait(console_visible, "docked Inspector Console tab")

    def click_prompt():
        height = story.picture(window).height
        story.click(window, 32 * scale, height - 14 * scale)

    def focus_prompt():
        if prompt_focused(story.picture(window), scale):
            return True
        click_prompt()
        return False

    click_prompt()
    story.wait(focus_prompt, f"{mode} Inspector prompt to gain keyboard focus")
    prompt = (0, image.height - 30 * scale, image.width // 2, image.height)
    empty = story.picture(window).crop(prompt)
    request = f"{FIXTURE}?inspector={mode}"
    prior = story.requests(request)
    story.set_clipboard(f"void fetch('?inspector={mode}')")
    # Reacquire the editor after preparing the clipboard and before sending Ctrl+V.
    click_prompt()
    story.wait(focus_prompt, f"{mode} Inspector prompt to regain keyboard focus")
    story.key("ctrl+v")

    def command_ready():
        changes = ImageChops.difference(empty, story.picture(window).crop(prompt)).getbbox()
        return changes and changes[2] - changes[0] >= 40 * scale

    story.wait(command_ready, f"{mode} command pasted into the Console prompt")
    story.key("Return")
    story.wait(lambda: story.requests(request) > prior, f"{mode} Inspector Console request")
    story.move_pointer(window, 4, 4)


def painted(story, window, condition):
    image = story.picture(window)
    return image if condition(image) else None


def run(story):
    browser = story.open(FIXTURE)
    scale = story.scale

    def is_full_page():
        image = story.picture(browser)
        return viewport_height(image, scale) == image.height

    story.wait(is_full_page, "inspector fixture painted")
    original_clients = story.clients()
    story.key("F12")
    top = story.wait(
        lambda: docked_top(story.picture(browser), scale),
        "docked Inspector frontend painted",
    )
    prove_console(story, browser, "docked", top)
    docked_frame = story.wait(
        lambda: painted(story, browser, lambda image: docked_top(image, scale)),
        "docked Inspector after its Console request",
    )
    story.save(docked_frame, "inspector-docked")

    # The stock WebKitGTK bottom dock toolbar places Undock after Close, Left and Right.
    click(story, browser, 88 * scale, top + 14 * scale)
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
    prove_console(story, detached, "detached", 50 * scale)
    detached_frame = story.wait(
        lambda: painted(story, detached, lambda image: has_frontend(image, 50 * scale, scale)),
        "detached Inspector after its Console request",
    )
    story.save(detached_frame, "inspector-detached")

    story.key("F12")
    story.wait(lambda: detached not in story.clients(), "F12 closes detached Inspector")
