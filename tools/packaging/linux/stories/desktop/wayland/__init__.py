"""Native Wayland story desktop, with an isolated Sway seat and no XWayland."""

import json
import re
import subprocess

from ..process import decode, start, stop, wait
from .pointer import attach_pointer


def window_nodes(node):
    if node.get("shell"):
        yield node
    for kind in ("nodes", "floating_nodes"):
        for child in node.get(kind, ()):
            yield from window_nodes(child)


class Wayland:
    def __init__(self, stack, directory, environment, scale):
        self.environment = environment
        self.scale = scale
        self.clipboard = None
        for name in ("DISPLAY", "GDK_BACKEND", "SWAYSOCK", "WAYLAND_DISPLAY", "WAYLAND_SOCKET"):
            self.environment.pop(name, None)
        self.environment.update(
            XDG_SESSION_TYPE="wayland", WLR_BACKENDS="headless", WLR_RENDERER="pixman",
            WLR_HEADLESS_OUTPUTS="1", WLR_LIBINPUT_NO_DEVICES="1",
        )
        width, height = (2640, 1740) if scale == 2 else (1440, 960)
        config = directory / "sway.conf"
        config.write_text(
            "xwayland disable\n"
            "swaybg_command -\n"
            "default_border none\n"
            "default_floating_border none\n"
            "focus_follows_mouse no\n"
            "for_window [shell=\"xdg_shell\"] floating enable\n"
            f"output HEADLESS-1 mode {width}x{height} scale {scale}\n"
        )
        log = stack.enter_context((directory / "sway.log").open("w+"))
        compositor = start(
            stack, "sway", "--config", str(config), env=self.environment,
            stdout=log, stderr=subprocess.STDOUT,
        )

        def sockets():
            if compositor.poll() is not None:
                log.seek(0)
                raise RuntimeError(f"Sway exited with {compositor.returncode}: {log.read()}")
            runtime = directory / "runtime"
            ipc = list(runtime.glob("sway-ipc.*.sock"))
            displays = [p.name for p in runtime.iterdir() if re.fullmatch(r"wayland-\d+", p.name)]
            if len(ipc) == 1 and len(displays) == 1:
                return str(ipc[0]), displays[0]
            return None

        socket, display = wait(sockets, "the isolated Sway sockets")
        self.environment.update(SWAYSOCK=socket, WAYLAND_DISPLAY=display)

        def output_ready():
            for output in self.ipc("get_outputs"):
                if output["name"] == "HEADLESS-1" and output["active"]:
                    mode = output.get("current_mode") or {}
                    return (mode.get("width"), mode.get("height"), output["scale"]) == (width, height, scale)
            return False

        wait(output_ready, f"active HEADLESS-1 at {width}x{height} scale {scale}")
        # Keep the seat's keyboard capability advertised between wtype invocations.
        # Otherwise a client can receive the first keystroke before subscribing to the seat.
        start(stack, "wtype", "-s", "2147483647", env=self.environment)
        attach_pointer(stack, directory / "runtime" / display)
        wait(
            lambda: {"keyboard", "pointer"} <= {item["type"] for item in self.ipc("get_inputs")},
            "the isolated Wayland seat input devices",
        )

    def ipc(self, kind, *command):
        return json.loads(subprocess.check_output(
            ["swaymsg", "-r", "-t", kind, *command], env=self.environment,
        ))

    def command(self, command):
        for reply in self.ipc("command", command):
            if not reply["success"]:
                raise RuntimeError(f"Sway rejected {command!r}: {reply.get('error')}")

    def windows(self):
        windows = {}
        for window in window_nodes(self.ipc("get_tree")):
            if window["shell"] != "xdg_shell":
                raise RuntimeError(f"Expected native Wayland, found {window['shell']}")
            windows[str(window["id"])] = window
        return windows

    def clients(self):
        return set(self.windows())

    def window_title(self, window):
        return self.windows()[window]["name"]

    def focus(self, window):
        self.command(f"[con_id={window}] focus")
        wait(lambda: self.windows()[window]["focused"], "the Wayland window to acquire focus")

    def close(self, window):
        self.command(f"[con_id={window}] kill")

    def key(self, *keys):
        command = ["wtype"]
        for key in keys:
            *modifiers, symbol = key.split("+")
            for modifier in modifiers:
                command.extend(("-M", modifier))
            command.extend(("-k", symbol))
            for modifier in reversed(modifiers):
                command.extend(("-m", modifier))
        subprocess.run(command, env=self.environment, check=True)

    def paste(self, stack, text):
        if self.clipboard is not None:
            stop(self.clipboard)
        self.clipboard = start(stack, "wl-copy", "--foreground", "--", text, env=self.environment)
        wait(
            lambda: subprocess.check_output(
                ["wl-paste", "--no-newline"], env=self.environment, stderr=subprocess.DEVNULL,
            ).decode() == text,
            "the native Wayland clipboard selection",
        )
        self.key("ctrl+v")

    def picture(self, window):
        rect = self.windows()[window]["rect"]
        geometry = f"{rect['x']},{rect['y']} {rect['width']}x{rect['height']}"
        return decode(subprocess.check_output(
            ["grim", "-s", str(self.scale), "-g", geometry, "-"], env=self.environment,
        ))

    def resize(self, window, width, height):
        self.command(f"[con_id={window}] resize set width {width} px height {height} px")

    def move_pointer(self, window, x, y):
        rect = self.windows()[window]["rect"]
        self.command(f"seat seat0 cursor set {rect['x'] + x // self.scale} {rect['y'] + y // self.scale}")

    def click(self, window, x, y):
        self.move_pointer(window, x, y)
        self.command("seat seat0 cursor press button1; seat seat0 cursor release button1")
