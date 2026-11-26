"""Exercise the GTK MCP socket host, consent modal, and standalone CLI fallback."""

import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
from threading import Thread

from PIL import ImageChops

from ..story import viewport_height

FIXTURE = "inspector/fixture.html"


def _send(proc, message):
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()


def _recv(proc):
    line = proc.stdout.readline()
    if not line:
        stderr = proc.stderr.read()
        raise RuntimeError(f"MCP process closed stdout unexpectedly: {stderr}")
    return json.loads(line)


def _initialize(proc):
    _send(
        proc,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "gtk-story-mcp", "version": "1.0"},
            },
        },
    )
    reply = _recv(proc)
    assert reply["result"]["serverInfo"]["name"] == "quickie"
    _send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})


def _verify_standalone(story):
    with TemporaryDirectory() as tmp:
        runtime = Path(tmp) / "runtime"
        runtime.mkdir(mode=0o700)
        env = story.environment.copy()
        env["XDG_RUNTIME_DIR"] = str(runtime)
        proc = subprocess.Popen(
            [str(story.binary), "--mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
        try:
            _initialize(proc)
            _send(
                proc,
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {"name": "browser_status", "arguments": {}},
                },
            )
            status = _recv(proc)["result"]["structuredContent"]
            assert status["platform"] == "linux"
            assert status["runtime_mode"] == "standalone"
            assert status["fork_capabilities"]["agent_automation"] is False
        finally:
            proc.stdin.close()
            assert proc.wait(timeout=10) == 0


def run(story):
    _verify_standalone(story)

    window = story.open(FIXTURE)

    def fixture_painted():
        image = story.picture(window)
        return image if viewport_height(image, story.scale) == image.height else None

    baseline = story.wait(fixture_painted, "inspected fixture painted")
    center = (baseline.width // 4, baseline.height // 4,
              3 * baseline.width // 4, 3 * baseline.height // 4)
    baseline_center = baseline.crop(center)

    proc = subprocess.Popen(
        [str(story.binary), "mcp", "--attach-only"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=story.environment,
    )
    try:
        _initialize(proc)
        _send(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "browser_status", "arguments": {}},
            },
        )
        status = _recv(proc)["result"]["structuredContent"]
        assert status["platform"] == "linux"
        assert status["runtime_mode"] == "gui"

        if not status["fork_capabilities"]["diagnostic"]["console"]:
            _send(
                proc,
                {
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tools/call",
                    "params": {"name": "diagnostic_request", "arguments": {}},
                },
            )
            denied = _recv(proc)["result"]
            assert denied["isError"] is True
            assert denied["structuredContent"]["code"] == "unsupported_capability"
            story.save(baseline, "mcp-stock-unavailable")
            return

        replies = []

        def read_grant():
            replies.append(_recv(proc))

        reader = Thread(target=read_grant, daemon=True)
        reader.start()
        _send(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "diagnostic_request",
                    "arguments": {"scope": "single_document"},
                },
            },
        )

        def consent_visible():
            if replies:
                result = replies[0]["result"]["structuredContent"]
                raise AssertionError(f"MCP replied before consent ({result.get('code')})")
            current = story.picture(window)
            diff = ImageChops.difference(baseline_center, current.crop(center)).getbbox()
            return current if diff is not None else None

        prompt_image = story.wait(consent_visible, "MCP consent modal")
        story.save(prompt_image, "mcp-consent-prompt")

        story.key("1")
        reader.join(timeout=10)
        assert replies, "diagnostic_request did not complete after approval"
        grant = replies[0]["result"]["structuredContent"]
        assert "attachment_id" in grant, f"MCP consent failed ({grant.get('code')})"
        attachment_id = grant["attachment_id"]
        assert grant["origin"].startswith("http://127.0.0.1:")

        story.save(story.picture(window), "mcp-diagnostics-active")

        _send(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "page_snapshot",
                    "arguments": {
                        "target": {"kind": "diagnostic", "attachment_id": attachment_id},
                        "kind": "text",
                    },
                },
            },
        )
        snapshot = _recv(proc)["result"]["structuredContent"]
        assert snapshot["kind"] == "text"

        _send(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "diagnostic_detach",
                    "arguments": {"attachment_id": attachment_id},
                },
            },
        )
        detached = _recv(proc)["result"]["structuredContent"]
        assert detached["detached"] is True
    finally:
        proc.stdin.close()
        assert proc.wait(timeout=10) == 0
        story.close(window)
