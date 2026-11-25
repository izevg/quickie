"""Open the GTK workspace and submit real turns against an isolated loopback endpoint."""

from contextlib import closing
from pathlib import Path
import sqlite3

from PIL import Image, ImageChops

from .endpoint import Endpoint

NATIVE_PROBES = ["/v1/chat/completions", "/v1/responses"]


def run(story):
    endpoint = Endpoint(story.stack)
    window = story.open("agent/fixture.html")
    scale = story.scale

    def picture():
        return story.picture(window)

    def marker(image, right):
        return image.getpixel((image.width - right * scale, image.height - 18 * scale))

    def page_ready():
        image = picture()
        return image if marker(image, 18) == (95, 226, 176) else None

    def workspace():
        image = picture()
        return image if marker(image, 382) == (95, 226, 176) else None

    def region(image, top, height):
        return image.crop((image.width - 347 * scale, top * scale, image.width - 18 * scale,
                           (top + height) * scale))

    def contrast_changed(before, after):
        delta = ImageChops.difference(before, after)
        bounds = delta.convert("L").point(lambda level: 255 if level > 36 else 0).getbbox()
        return bounds is not None and bounds[2] - bounds[0] >= 14 * scale

    def changed(before, top, height):
        image = picture()
        return image if contrast_changed(region(before, top, height), region(image, top, height)) else None

    def settings_heading_is_visible(image):
        # Only the second word of the Settings heading renders in this part of the pane.
        suffix = image.crop((image.width - 280 * scale, 23 * scale,
                             image.width - 192 * scale, 42 * scale))
        background = image.getpixel((image.width - 280 * scale, 20 * scale))
        flat = Image.new("RGB", suffix.size, background)
        return ImageChops.difference(suffix, flat).convert("L").getextrema()[1] > 40

    def open_command(name):
        def entry(image):
            center = image.width // 2
            return image.crop((center - 325 * scale, 171 * scale, center + 100 * scale, 198 * scale))

        before = entry(picture())
        story.key("ctrl+shift+p")

        def palette_ready():
            visible = entry(picture())
            return visible if contrast_changed(before, visible) else None

        blank = story.wait(palette_ready, "the command palette is ready for input")
        story.paste(name)
        story.wait(lambda: contrast_changed(blank, entry(picture())), "the palette receives the pasted command")
        story.key("Return")

    def submit(text, count):
        before = picture()
        story.paste(text)
        story.wait(lambda: changed(before, 693, 16), "the composer receives the pasted prompt")
        story.key("Return")
        return story.wait(lambda: len(endpoint.snapshot()) == count and endpoint.snapshot()[-1],
                          f"the endpoint receives turn {count}")

    def click(right, y):
        story.click(window, picture().width - right * scale, y * scale)

    story.wait(page_ready, "WebKit renders the source fixture")
    assert not endpoint.snapshot() and not endpoint.probe_paths(), "a fresh browser must leave the endpoint idle"
    open_command("Ask Agent About This Page")
    unconfigured = story.wait(workspace, "Agent opens beside the source page")
    assert not endpoint.snapshot() and not endpoint.probe_paths(), "opening Agent must leave the endpoint idle"
    story.save(unconfigured, "agent-workspace-unconfigured")

    click(115, 33)
    def settings_open():
        image = picture()
        return image if settings_heading_is_visible(image) else None

    form = story.wait(settings_open, "Agent Settings opens")
    story.save(form, "agent-settings")
    story.paste(endpoint.address)
    story.wait(lambda: changed(form, 96, 17), "the endpoint field receives the URL")
    story.key("Tab")
    story.paste("observatory-fixture")
    story.wait(lambda: changed(form, 168, 17), "the model field receives the model")
    click(312, 293)
    saved = story.wait(lambda: changed(form, 321, 29), "the endpoint is saved")
    story.save(saved, "agent-settings-saved")
    assert not endpoint.snapshot() and not endpoint.probe_paths(), "saving the endpoint must leave it idle"

    story.key("Escape")
    def workspace_restored():
        image = picture()
        return image if not settings_heading_is_visible(image) else None

    idle = story.wait(workspace_restored, "Escape returns to the workspace")
    story.save(idle, "agent-workspace-idle")
    request = submit("When does the observatory open?", 1)
    assert endpoint.probe_paths() == NATIVE_PROBES, "both explicit rejections enable streaming compatibility"
    assert request["model"] == "observatory-fixture"
    contents = " ".join(message["content"] for message in request["messages"])
    assert "The northern observatory opens at dusk." in contents
    assert "untrusted data" in contents
    assert "This hidden material" not in contents
    assert "Page script supplied text" not in contents
    success = story.wait(lambda: changed(idle, 145, 132), "the transcript shows the sourced answer")
    story.save(success, "agent-workspace-success")

    passage = (60 * scale, 122 * scale, 340 * scale, 151 * scale)
    before_citation = success.crop(passage)
    click(170, 240)

    def source_highlighted():
        image = picture()
        return image if ImageChops.difference(before_citation, image.crop(passage)).getbbox() else None

    citation = story.wait(source_highlighted, "the verified source chip highlights the matching passage")
    story.save(citation, "agent-workspace-citations")
    story.key("Escape")
    click(180, 701)

    submit("Repeat the opening time", 2)
    history = story.wait(lambda: changed(success, 210, 150), "the second turn extends the transcript")
    story.save(history, "agent-workspace-history")

    submit("Keep this answer open", 3)
    story.wait(endpoint.stopping.is_set, "the loopback endpoint starts a pending answer")
    running = story.wait(lambda: changed(history, 300, 190), "the active turn is visible")
    story.save(running, "agent-workspace-running")
    click(235, 747)
    story.wait(endpoint.disconnected.is_set, "Stop disconnects the pending HTTP transfer")
    stopped = story.wait(lambda: changed(running, 484, 74), "Stop updates the transcript")
    story.save(stopped, "agent-workspace-stopped")

    submit("Return a service failure", 4)
    error = story.wait(lambda: changed(stopped, 380, 190), "a failed endpoint updates the transcript")
    story.save(error, "agent-workspace-error")

    story.click(window, 140 * scale, 188 * scale)
    story.wait(lambda: marker(picture(), 382) == (226, 169, 95), "the source page remains clickable")
    click(41, 33)
    story.wait(lambda: marker(picture(), 18) == (226, 169, 95), "Close restores the source page width")
    assert len(endpoint.snapshot()) == 4
    assert endpoint.probe_paths() == NATIVE_PROBES, "the workspace reuses its proven streaming route"

    # The real settings load takes an immediate SQLite transaction during store opening.
    # Holding that write lock keeps the first palette Ask queued while the page changes.
    database = Path(story.environment["XDG_DATA_HOME"]) / "quickie" / "shell.sqlite"
    with closing(sqlite3.connect(database, timeout=2)) as lock:
        lock.execute("BEGIN IMMEDIATE")
        open_command("? Read the original observatory")
        story.wait(lambda: marker(picture(), 382) == (226, 169, 95), "the palette Ask opens the workspace")
        story.wait(lambda: changed(idle, 693, 16), "the original Ask is queued in the composer")
        assert len(endpoint.snapshot()) == 4, "settings must still be pending"
        assert endpoint.probe_paths() == NATIVE_PROBES, "settings must finish before any native probe"
        story.click(window, 350 * scale, 183 * scale)
        story.wait(lambda: story.requests("agent/fixture.html?replacement"), "WebKit requests a new document")

        def replacement_painted():
            image = picture()
            return image if marker(image, 382) == (159, 189, 255) else None

        pending = story.wait(replacement_painted, "the new document finishes painting")
        lock.rollback()

    story.wait(lambda: changed(pending, 646, 30), "the settings load ends after the navigation")
    assert len(endpoint.snapshot()) == 4, "the queued Ask sent a request after its source changed"
    assert endpoint.probe_paths() == NATIVE_PROBES, "the stale Ask must leave both native routes idle"
    click(180, 701)
    replacement = submit("Read the replacement observatory", 5)
    contents = " ".join(message["content"] for message in replacement["messages"])
    assert "Read the replacement observatory" in contents, "the endpoint received the stale queued Ask"
    assert "Read the original observatory" not in contents
    assert "The southern observatory opens at sunrise." in contents
    assert "The northern observatory opens at dusk." not in contents
    assert endpoint.probe_paths() == NATIVE_PROBES * 2, "a new workspace detects the endpoint again"
    story.save(story.wait(lambda: changed(pending, 175, 122), "the workspace shows the new answer"),
               "agent-workspace-source-changed")

    click(41, 33)
    story.wait(lambda: marker(picture(), 18) == (159, 189, 255), "the replacement page fills the window")
    with closing(sqlite3.connect(database, timeout=2)) as lock:
        lock.execute("BEGIN IMMEDIATE")
        open_command("? Read the replacement observatory again")
        story.wait(replacement_painted, "the next palette Ask opens on the unchanged page")
        story.wait(lambda: changed(idle, 693, 16), "the new Ask is queued in the composer")
        assert len(endpoint.snapshot()) == 5, "settings must still be pending on the unchanged page"
        lock.rollback()

    fresh = story.wait(lambda: len(endpoint.snapshot()) == 6 and endpoint.snapshot()[-1],
                       "the unchanged document allows the new queued Ask")
    contents = " ".join(message["content"] for message in fresh["messages"])
    assert "Read the replacement observatory again" in contents
    assert "The southern observatory opens at sunrise." in contents
    assert endpoint.probe_paths() == NATIVE_PROBES * 3
