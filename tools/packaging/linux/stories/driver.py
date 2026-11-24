"""Drive the real GTK browser in an isolated desktop session and capture its stories."""

import argparse
from collections import Counter
from contextlib import ExitStack
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from importlib import import_module
import json
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from threading import Lock, Thread
from desktop.process import start, wait
from desktop.wayland import Wayland
from desktop.x11 import X11


STORIES = Path(__file__).resolve().parent
GROUPS = (
    "palette", "find", "inspector", "dialogs", "tabs", "privacy", "agent", "agent-providers",
    "media", "downloads", "settings", "keyboard", "hints", "origin",
)
NESTED_GROUPS = {
    "agent-providers": "agent.providers",
    "media": "tabs.media",
    "downloads": "tabs.downloads",
    "settings": "tabs.settings",
    "keyboard": "tabs.settings.keyboard",
    "hints": "tabs.hints",
    "origin": "tabs.origin",
}


class Fixture(SimpleHTTPRequestHandler):
    def do_GET(self):
        with self.server.request_lock:
            self.server.requests[self.path] += 1
        super().do_GET()

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, *_args):
        pass


class Story:
    def __init__(self, stack, binary, output, desktop, profile):
        self.stack = stack
        self.binary = binary
        self.output = output
        self.desktop = desktop
        self.processes = {}
        self.scale = desktop.scale
        self.environment = desktop.environment.copy()
        for leaf in ("cache", "config", "data"):
            directory = profile / leaf
            directory.mkdir(mode=0o700, parents=True)
            self.environment[f"XDG_{leaf.upper()}_HOME"] = str(directory)
        self.output.mkdir(parents=True, exist_ok=True)
        self.server = stack.enter_context(
            ThreadingHTTPServer(("127.0.0.1", 0), partial(Fixture, directory=str(STORIES)))
        )
        self.server.requests = Counter()
        self.server.request_lock = Lock()
        worker = Thread(target=self.server.serve_forever, daemon=True)
        worker.start()
        stack.callback(worker.join)
        stack.callback(self.server.shutdown)

    def url(self, path):
        return f"http://127.0.0.1:{self.server.server_port}/{path.lstrip('/')}"

    def requests(self, path):
        with self.server.request_lock:
            return self.server.requests["/" + path.lstrip("/")]

    def key(self, *keys):
        self.desktop.key(*keys)

    def set_clipboard(self, text):
        self.desktop.set_clipboard(self.stack, text)

    def paste(self, text):
        self.set_clipboard(text)
        self.key("ctrl+v")

    def clients(self):
        return self.desktop.clients()

    def window_title(self, window):
        return self.desktop.window_title(window)

    def focus(self, window):
        self.desktop.focus(window)

    def move_pointer(self, window, x, y):
        self.desktop.move_pointer(window, x, y)

    def click(self, window, x, y):
        self.desktop.click(window, x, y)

    def open(self, path=None):
        log = self.stack.enter_context((self.output / "browser.log").open("w"))
        previous = self.clients()
        arguments = () if path is None else (self.url(path),)
        process = start(
            self.stack, str(self.binary), *arguments, env=self.environment,
            stdout=log, stderr=subprocess.STDOUT,
        )

        def window():
            if process.poll() is not None:
                raise RuntimeError(f"Quickie exited with {process.returncode}; see {log.name}")
            for identifier in sorted(self.clients() - previous):
                if self.window_title(identifier) == "Quickie":
                    return identifier
            return None

        identifier = self.wait(window, "Quickie window")
        self.processes[identifier] = process
        self.focus(identifier)
        self.move_pointer(identifier, 4, 4)
        return identifier

    def close(self, window):
        process = self.processes[window]
        self.desktop.close(window)
        self.wait(
            lambda: process.poll() is not None and window not in self.clients(),
            "Quickie to close",
        )
        if process.returncode != 0:
            raise RuntimeError(f"Quickie exited with {process.returncode}")
        self.processes.pop(window)

    def picture(self, window):
        return self.desktop.picture(window)

    def capture(self, window, name):
        return self.save(self.picture(window), name)

    def save(self, image, name):
        image.save(self.output / f"{name}.png")
        return image

    def resize(self, window, width, height):
        self.desktop.resize(window, width, height)

    wait = staticmethod(wait)


def render(args):
    groups = args.group or GROUPS
    outputs = {
        group: args.output / group if len(groups) > 1 else args.output
        for group in groups
    }
    for output in outputs.values():
        (output / "passed.json").unlink(missing_ok=True)
    if not os.environ.get("DBUS_SESSION_BUS_ADDRESS"):
        raise RuntimeError("Run the GTK stories in a dbus-run-session")
    sys.dont_write_bytecode = True
    binary = args.binary.resolve(strict=True)
    with ExitStack() as session:
        config = Path(session.enter_context(TemporaryDirectory()))
        runtime = config / "runtime"
        runtime.mkdir(mode=0o700)
        environment = os.environ.copy()
        environment.update(
            XDG_RUNTIME_DIR=str(runtime), GTK_A11Y="none",
            GTK_THEME="Adwaita:dark" if args.theme == "dark" else "Adwaita",
            LIBGL_ALWAYS_SOFTWARE="1", MESA_SHADER_CACHE_DISABLE="true",
        )
        environment.pop("GDK_SCALE", None)
        if args.backend == "wayland":
            desktop = Wayland(session, config, environment, args.scale)
        else:
            desktop = X11(
                session, config, environment, args.scale, args.display, STORIES / "clipboard.py",
            )
        for group, output in outputs.items():
            with ExitStack() as scope:
                story = Story(scope, binary, output, desktop, config / group)
                status = output / "passed.json"
                try:
                    module = NESTED_GROUPS.get(group, group)
                    import_module(f"{module}.story").run(story)
                except Exception:
                    for window in sorted(story.clients()):
                        try:
                            story.capture(window, f"failure-window-{window}")
                        except subprocess.CalledProcessError:
                            continue
                    raise
                result = {
                    "backend": args.backend, "group": group, "scale": args.scale, "theme": args.theme,
                }
                if group == "privacy":
                    result["mock_capture_devices"] = story.environment.get("QUICKIE_GTK_STORY_MEDIA") == "1"
                status.write_text(json.dumps(result) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--group", action="append", choices=GROUPS)
    parser.add_argument("--theme", choices=("light", "dark"), default="light")
    parser.add_argument("--scale", type=int, choices=(1, 2), default=1)
    parser.add_argument("--backend", choices=("wayland", "x11"), default="wayland")
    parser.add_argument("--display", help="display number for X11 compatibility runs")
    arguments = parser.parse_args()
    if arguments.display is not None and arguments.backend != "x11":
        parser.error("--display requires --backend x11")
    render(arguments)
