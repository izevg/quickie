"""Drive the native Shields and media permission surfaces with a loopback fixture."""

from collections import Counter
from pathlib import Path
import sqlite3
import time
from urllib.parse import urlencode

from PIL import ImageChops

FIXTURE = "privacy/fixture.html"
CONTROL = "privacy/probe.svg?role=control"
TRACKER = "privacy/probe.svg?role=tracker"
READY = (95, 226, 176)
NO_NETWORK_RULES_ACTIVE = "No network rules active"
WEBKIT_DOC_ROLES = frozenset({"document web", "document frame", "embedded"})


def result(scene, answer):
    return FIXTURE + "?" + urlencode({"scene": scene, "result": answer})


def marker(image, scale):
    return image.getpixel((image.width - 18 * scale, image.height - 18 * scale))


def ready(story, window):
    def painted():
        image = story.picture(window)
        return image if marker(image, story.scale) == READY else None

    return story.wait(painted, "privacy fixture ready")


def changed_frame(story, window, before, area, label):
    def changed():
        image = story.picture(window)
        bounds = ImageChops.difference(before.crop(area), image.crop(area)).getbbox()
        if bounds and bounds[2] - bounds[0] > 120 * story.scale and bounds[3] - bounds[1] > 35 * story.scale:
            return image
        return None

    return story.wait(changed, label)


def shields(story, window, before):
    story.key("ctrl+alt+s")
    return changed_frame(
        story, window, before,
        (before.width // 2, 0, before.width, before.height * 4 // 5),
        "styled native Shields popover",
    )


def settings(story, window, before):
    # The first tab stop is the site's Shields switch, followed by its other two switches.
    story.key("Tab", "Tab", "Tab", "Tab", "space")
    return changed_frame(
        story, window, before,
        (before.width // 4, 0, before.width * 3 // 4, before.height),
        "styled global Shields preferences",
    )


def profile(story):
    return Path(story.environment["XDG_DATA_HOME"]) / "quickie" / "shell.sqlite"


def seed(story, *, corrupt=False):
    # Use the real profile created by Quickie. The recorded hash uses the domain's FNV-1a.
    body = (
        b"[Adblock Plus 2.0]\n"
        b"/privacy/probe.svg?role=tracker$image\n"
        b"127.0.0.1##[broken-selector\n"
        b"127.0.0.1##.quickie-host-ad\n"
        b"##.quickie-generic-ad\n"
        b"127.0.0.1##.quickie-procedural-ad:has-text(Sponsored)\n"
        b"127.0.0.1##+js(json-prune, adPlacements)\n"
        b"127.0.0.1##+js(trusted-set-constant, quickieFixtureTrusted, true)\n"
    )
    fingerprint = 0xCBF29CE484222325
    for byte in body:
        fingerprint = ((fingerprint ^ byte) * 0x100000001B3) & ((1 << 64) - 1)
    if fingerprint >= 1 << 63:
        fingerprint -= 1 << 64
    with sqlite3.connect(profile(story)) as database:
        database.execute(
            """INSERT OR REPLACE INTO filter_lists VALUES ('easylist', 1, ?, ?)""",
            (time.time_ns() // 1_000_000, fingerprint),
        )
        database.execute(
            """INSERT OR REPLACE INTO filter_list_bodies VALUES ('easylist', ?)""",
            (b"Corrupt bytes" if corrupt else body,),
        )
        database.execute("DELETE FROM site_policies")


def store_value(story, sql):
    with sqlite3.connect(profile(story)) as database:
        row = database.execute(sql).fetchone()
    return row[0] if row else None


def cosmetic_reports(story):
    prefix = "/" + result("shields", "cosmetics-")
    with story.server.request_lock:
        return Counter({
            path.removeprefix(prefix): count
            for path, count in story.server.requests.items()
            if path.startswith(prefix)
        })


def observe(story, action, blocked):
    observation = result("shields", "loaded-blocked" if blocked else "loaded-loaded")
    opposite = result("shields", "loaded-loaded" if blocked else "loaded-blocked")
    before_result = story.requests(observation)
    before_opposite = story.requests(opposite)
    controls = story.requests(CONTROL)
    trackers = story.requests(TRACKER)
    expected_cosmetic = "1111111" if blocked else "0000111"
    previous_cosmetics = cosmetic_reports(story)
    action()
    story.wait(
        lambda: story.requests(observation) > before_result or story.requests(opposite) > before_opposite,
        "the fixture reports both image outcomes",
    )
    assert story.requests(observation) > before_result, (
        f"Control loaded, but the matching tracker was {'allowed' if blocked else 'blocked'}"
    )
    assert story.requests(CONTROL) > controls, "The allowed control must reach the loopback server"
    assert (story.requests(TRACKER) == trackers) == blocked, "The matching resource must follow the live native filter"
    observed = story.wait(
        lambda: next(iter(cosmetic_reports(story) - previous_cosmetics), None),
        "the fixture reports its cosmetic-filter outcomes",
    )
    labels = ("host CSS", "generic CSS", "procedural CSS", "scriptlet",
              "visible control", "isolated bridge", "rejected trusted scriptlet")
    assert observed == expected_cosmetic, (
        f"Cosmetics expected {expected_cosmetic}, observed {observed} in order {labels}"
    )


def open_page(story, blocked):
    windows = []
    observe(story, lambda: windows.append(story.open(FIXTURE)), blocked)
    return windows[0]


def wait_for_native_label(story, expected):
    import gi

    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi, GLib

    Atspi.init()
    Atspi.set_timeout(400, 400)
    desktop = Atspi.get_desktop(0)
    desktop.set_cache_mask(Atspi.Cache.NONE)

    def find():
        pending = [desktop]
        while pending:
            node = pending.pop()
            if node is None:
                continue
            try:
                role = node.get_role_name() or ""
                if role in WEBKIT_DOC_ROLES:
                    continue
                if role == "label" and (node.get_name() or "") == expected:
                    return True
                pending.extend(
                    node.get_child_at_index(index) for index in range(node.get_child_count())
                )
            except GLib.Error:
                continue
        return None

    return story.wait(find, f"native Settings label {expected!r}")


def settings_and_site(story, window):
    page = ready(story, window)
    protected = shields(story, window, page)
    story.save(protected, "shields-popover-protected")
    story.key("Tab", "space")
    story.wait(
        lambda: store_value(story, "SELECT shields FROM site_policies") == 0,
        "site exception committed to profile",
    )
    reload_frame = changed_frame(
        story, window, protected,
        (page.width // 2, 0, page.width, page.height),
        "Reload to Apply becomes available",
    )
    story.save(reload_frame, "shields-popover-reload")
    observe(story, lambda: story.key("Return"), False)
    story.close(window)

    window = open_page(story, False)
    page = ready(story, window)
    shields(story, window, page)
    story.key("Tab", "space")
    story.wait(
        lambda: store_value(story, "SELECT shields FROM site_policies") is None,
        "default site policy restored in profile",
    )
    observe(story, lambda: story.key("Return"), True)
    page = ready(story, window)
    protected = shields(story, window, page)
    story.save(settings(story, window, protected), "settings-shields-shields")
    story.key("Tab", "space")
    story.wait(
        lambda: store_value(story, "SELECT enabled FROM filter_lists WHERE id = 'easylist'") == 0,
        "global EasyList switch committed to profile",
    )
    wait_for_native_label(story, NO_NETWORK_RULES_ACTIVE)
    story.key("Escape")
    observe(story, lambda: story.key("ctrl+r"), False)
    story.close(window)


def permission_sheet(story, window, scene):
    page = ready(story, window)
    story.key("F8")
    return changed_frame(
        story, window, page,
        (page.width // 5, page.height // 4, page.width * 4 // 5, page.height * 3 // 4),
        f"native permission sheet for {scene}",
    )


def permissions(story):
    for scene, tracks in (
        ("camera", "video"), ("microphone", "audio"), ("camera-and-microphone", "audio-video"),
        ("display", "video"), ("display-with-audio", "video"),
    ):
        window = open_permission_fixture(story, scene)
        shown = permission_sheet(story, window, scene)
        assert all(
            abs(channel * 2 - initial) <= 2
            for channel, initial in zip(marker(shown, story.scale), READY)
        ), "Permission sheet dims the page behind the request"
        denied = result(scene, "NotAllowedError")
        allowed = result(scene, "allowed-" + tracks)
        previous = story.requests(denied)
        story.key("Escape")
        story.wait(lambda: story.requests(denied) > previous, f"{scene} denial reported by the page")
        story.save(shown, f"permission-sheet-{scene}")
        story.close(window)

        window = open_permission_fixture(story, scene)
        permission_sheet(story, window, scene)
        story.key("Return")
        assert marker(story.picture(window), story.scale) != READY, "Return does not grant media access"
        previous = story.requests(allowed)
        story.key("1")
        story.wait(lambda: story.requests(allowed) > previous, f"{scene} grant returns {tracks} tracks")
        if scene == "display":
            permission_sheet(story, window, scene)
            previous = story.requests(denied)
            story.key("Escape")
            story.wait(lambda: story.requests(denied) > previous, "a new display request requires consent again")
        story.close(window)
    display_navigation(story)


def display_navigation(story):
    scene = "display-navigates"
    destination = result("display", "ready")
    refused = result("display", "NotAllowedError")
    stale_grant = result(scene, "allowed-video")
    previous_destination = story.requests(destination)
    previous_grant = story.requests(stale_grant)
    window = open_permission_fixture(story, scene)
    permission_sheet(story, window, scene)
    story.wait(lambda: story.requests(destination) > previous_destination, "fixture navigated with display consent open")
    ready(story, window)
    story.key("1")
    permission_sheet(story, window, "display")
    previous_denial = story.requests(refused)
    story.key("Escape")
    story.wait(lambda: story.requests(refused) > previous_denial, "only the new page can request consent")
    assert story.requests(stale_grant) == previous_grant, "navigation must not grant the previous document's request"
    story.close(window)


def open_permission_fixture(story, scene):
    signal = result(scene, "ready")
    previous = story.requests(signal)
    window = story.open(FIXTURE + "?scene=" + scene)
    story.wait(lambda: story.requests(signal) > previous, f"{scene} fixture ready")
    return window


def run(story):
    story.environment["GTK_A11Y"] = "atspi"
    window = story.open()
    blank = story.picture(window)
    story.save(shields(story, window, blank), "shields-popover-off-the-web")
    story.close(window)

    window = open_page(story, False)
    page = ready(story, window)
    empty = shields(story, window, page)
    story.save(empty, "shields-popover-lists-off")
    story.save(settings(story, window, empty), "settings-shields-shields-no-lists")
    story.close(window)

    seed(story)
    settings_and_site(story, open_page(story, True))

    seed(story, corrupt=True)
    window = open_page(story, False)
    page = ready(story, window)
    story.save(shields(story, window, page), "shields-popover-rules-failed")
    story.close(window)
    if story.environment.get("QUICKIE_GTK_STORY_MEDIA") == "1":
        permissions(story)
