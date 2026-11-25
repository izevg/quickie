"""Drive provider detection, read-only tools, and response replay through GTK."""

from contextlib import closing
import json
from pathlib import Path
import sqlite3

from PIL import Image, ImageChops

from .endpoint import (
    ANSWER, ANSWER_REASONING, AUTH_FAILURE, FIRST, FOLLOW_UP, FORBIDDEN, HELD,
    NEW_ORIGIN, READ_CALL, READ_REASONING, Endpoint,
)

CHAT = "/v1/chat/completions"
RESPONSES = "/v1/responses"
SOURCE = "agent/fixture.html"
REPLACEMENT = SOURCE + "?replacement"
NORTH = (95, 226, 176)
SOUTH = (159, 189, 255)


class Workspace:
    def __init__(self, story):
        self.story = story
        self.scale = story.scale
        self.window = story.open(SOURCE)
        story.wait(lambda: self.source_marker(18) == NORTH, "the original source page renders")
        self.open_from_palette()
        story.wait(lambda: self.source_marker(382) == NORTH, "the Agent workspace opens beside the page")

    def picture(self):
        return self.story.picture(self.window)

    def source_marker(self, right):
        image = self.picture()
        scale = self.scale
        return image.getpixel((image.width - right * scale, image.height - 18 * scale))

    def click_panel(self, right, y):
        self.story.click(self.window, self.picture().width - right * self.scale, y * self.scale)

    def open_from_palette(self):
        scale = self.scale

        def entry():
            image = self.picture()
            center = image.width // 2
            return image.crop((center - 325 * scale, 171 * scale, center + 100 * scale, 198 * scale))

        page = entry()
        self.story.key("ctrl+shift+p")

        def visible():
            current = entry()
            return current if differs(page, current, scale) else None

        blank = self.story.wait(visible, "the command palette appears")
        self.story.paste("Ask Agent About This Page")
        self.story.wait(lambda: differs(blank, entry(), scale), "the palette receives the Agent command")
        self.story.key("Return")

    def field(self, top, height):
        image = self.picture()
        scale = self.scale
        return image.crop((image.width - 347 * scale, top * scale, image.width - 18 * scale,
                           (top + height) * scale))

    def paste_field(self, text, top, height):
        before = self.field(top, height)
        self.story.paste(text)
        self.story.wait(lambda: differs(before, self.field(top, height), self.scale),
                        "the focused field receives the input")

    def settings_visible(self):
        image = self.picture()
        scale = self.scale
        heading = image.crop((image.width - 280 * scale, 23 * scale,
                              image.width - 192 * scale, 42 * scale))
        background = image.getpixel((image.width - 280 * scale, 20 * scale))
        return differs(heading, Image.new("RGB", heading.size, background), scale)

    def configure(self, address, first=False):
        self.click_panel(115, 33)
        self.story.wait(self.settings_visible, "Agent Settings opens")
        self.story.key("ctrl+a")
        self.paste_field(address, 96, 17)
        if first:
            self.story.key("Tab")
            self.paste_field("read-tools-fixture", 168, 17)
        self.click_panel(312, 293)
        database = Path(self.story.environment["XDG_DATA_HOME"]) / "quickie" / "shell.sqlite"
        with closing(sqlite3.connect(database)) as connection:
            def saved():
                row = connection.execute("SELECT base_url FROM agent_endpoint WHERE id = 1").fetchone()
                return row == (address + "/",)

            self.story.wait(saved, "the selected endpoint is saved")
        self.story.wait(lambda: self.enabled_button(285), "the settings form receives its saved state")
        self.click_panel(40, 33)
        self.story.wait(lambda: not self.settings_visible(), "the composer returns after endpoint setup")
        self.settled()

    def enabled_button(self, y):
        image = self.picture()
        red, green, blue = image.getpixel((image.width - 333 * self.scale, y * self.scale))
        return image if blue > red + 25 and blue > green + 10 else None

    def settled(self):
        return self.story.wait(lambda: self.enabled_button(739), "the workspace settles and Send is enabled")

    def ask(self, prompt):
        self.click_panel(180, 701)
        self.paste_field(prompt, 693, 16)
        self.story.key("Return")

    def error_visible(self):
        return any(red > green + 45 and red > blue + 35 for red, green, blue in self.field(85, 550).getdata())


def differs(before, after, scale):
    delta = ImageChops.difference(before, after).convert("L")
    bounds = delta.point(lambda level: 255 if level > 36 else 0).getbbox()
    return bounds is not None and bounds[2] - bounds[0] >= 14 * scale


def request_count(story, endpoint, count, description):
    return story.wait(lambda: len(endpoint.snapshot()) >= count and endpoint.snapshot(), description)


def assert_read_only(request):
    tools = request["tools"]
    assert [tool["name"] for tool in tools] == ["read_page"]
    assert tools[0]["parameters"]["properties"]["page"]["enum"] == ["source"]
    assert request["stream"] is False
    assert request["store"] is False
    assert request["include"] == ["reasoning.encrypted_content"]
    assert {"session_id", "previous_response_id", "conversation"}.isdisjoint(request)


def run(story):
    first = Endpoint(story.stack)
    second = Endpoint(story.stack)
    workspace = Workspace(story)
    workspace.configure(first.address, first=True)
    assert not first.snapshot() and not second.snapshot(), "opening and configuring Agent leaves the endpoints idle"

    workspace.ask(FIRST)
    exchanges = request_count(story, first, 3, "a definite Chat route rejection switches to native Responses and reads the source")
    assert [route for route, _ in exchanges] == [CHAT, RESPONSES, RESPONSES]
    assert [tool["function"]["name"] for tool in exchanges[0][1]["tools"]] == ["read_page"]
    initial, continuation = exchanges[1][1], exchanges[2][1]
    assert_read_only(initial)
    assert_read_only(continuation)
    assert initial["input"][-1] == {"role": "user", "content": FIRST}
    assert "The northern observatory opens at dusk." not in json.dumps(initial["input"])
    source = continuation["input"][-1]
    assert source["type"] == "function_call_output" and source["call_id"] == "call-fixture-source"
    assert "The northern observatory opens at dusk." in source["output"]
    assert "This hidden material" not in source["output"] and "Page script supplied text" not in source["output"]
    assert continuation["input"] == initial["input"] + [READ_REASONING, READ_CALL, source]
    story.save(workspace.settled(), "provider-source-answer")

    workspace.ask(FOLLOW_UP)
    route, follow_up = request_count(story, first, 4, "the next turn starts on the cached Responses route")[-1]
    assert route == RESPONSES
    assert_read_only(follow_up)
    assert follow_up["input"] == continuation["input"] + [
        ANSWER_REASONING, ANSWER, {"role": "user", "content": FOLLOW_UP},
    ]
    story.save(workspace.settled(), "provider-stateless-replay")

    workspace.ask(FORBIDDEN)
    request_count(story, first, 5, "the read-only provider receives the next explicit question")
    story.wait(workspace.error_visible, "the browser rejects the unadvertised navigation tool")
    story.save(workspace.settled(), "provider-action-rejected")
    assert len(first.snapshot()) == 5, "a rejected action produces no tool continuation"
    assert story.requests(SOURCE) == 1 and story.requests(REPLACEMENT) == 0

    workspace.ask(AUTH_FAILURE)
    exchanges = request_count(story, first, 6, "the cached provider returns its authorization error")
    assert exchanges[-1][0] == RESPONSES
    workspace.settled()
    assert len(first.snapshot()) == 6, "authorization failure must not retry on either API"

    workspace.ask(HELD)
    request_count(story, first, 7, "the model step is pending before the document changes")
    story.click(workspace.window, 350 * workspace.scale, 183 * workspace.scale)
    story.wait(lambda: workspace.source_marker(382) == SOUTH, "the replacement document renders")
    first.release.set()
    story.wait(first.released_response.is_set, "the provider releases its source-read call")
    story.save(workspace.settled(), "provider-source-expired")
    assert len(first.snapshot()) == 7, "a stale turn must not send source data after navigation"

    workspace.configure(second.address)
    assert not second.snapshot(), "changing endpoint alone sends no model request"
    workspace.ask(NEW_ORIGIN)
    fresh = request_count(story, second, 2, "the new origin detects its own compatible route")
    assert [route for route, _ in fresh] == [CHAT, RESPONSES]
    assert_read_only(fresh[1][1])
    inputs = fresh[1][1]["input"]
    assert len(inputs) == 2 and inputs[0]["role"] == "system"
    assert inputs[1] == {"role": "user", "content": NEW_ORIGIN}
    assert len(fresh[0][1]["messages"]) == 2, "route probing must also discard the previous origin's context"
    assert len(first.snapshot()) == 7, "the previous endpoint receives no new requests"
    story.save(workspace.settled(), "provider-new-origin")
