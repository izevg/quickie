"""X11 compatibility desktop for the shared GTK stories."""

import re
import select
import subprocess
import sys

from .keyboard import press_f12
from .process import decode, start, stop, wait


class X11:
    def __init__(self, stack, directory, environment, scale, display, clipboard_script):
        self.environment = environment
        self.scale = scale
        self.clipboard = None
        self.clipboard_script = clipboard_script
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "SWAYSOCK"):
            self.environment.pop(name, None)
        self.environment.update(GDK_BACKEND="x11", GDK_SCALE=str(scale), XDG_SESSION_TYPE="x11")
        log = stack.enter_context((directory / "x11.log").open("w+"))
        screen = "2640x1740x24" if scale == 2 else "1440x960x24"
        display_option = [display] if display is not None else []
        xvfb = start(
            stack, "Xvfb", *display_option, "-displayfd", "1", "-screen", "0", screen, "-nolisten", "tcp",
            env=self.environment, stdout=subprocess.PIPE, stderr=log, text=True,
        )
        stack.callback(xvfb.stdout.close)

        def ready_display():
            # Trust only this child's readiness message. Any other X server on the host
            # could answer an X11 probe while a colliding Xvfb child is still exiting.
            if select.select([xvfb.stdout], [], [], 0)[0]:
                number = xvfb.stdout.readline().strip()
                if number.isdecimal():
                    return f":{number}"
            if xvfb.poll() is not None:
                log.seek(0)
                raise RuntimeError(f"Xvfb exited with {xvfb.returncode}: {log.read()}")
            return None

        self.display = wait(ready_display, "the isolated Xvfb child's display number")
        self.environment["DISPLAY"] = self.display
        start(stack, "openbox", "--sm-disable", env=self.environment, stdout=log, stderr=subprocess.STDOUT)
        wait(lambda: "window id #" in self.xprop("_NET_SUPPORTING_WM_CHECK"), "Openbox")

    def xd(self, *args):
        return subprocess.check_output(
            ["xdotool", *map(str, args)], env=self.environment, text=True,
        ).strip()

    def xprop(self, property_name):
        return subprocess.check_output(
            ["xprop", "-root", property_name], env=self.environment, text=True,
        )

    def clients(self):
        identifiers = self.xprop("_NET_CLIENT_LIST")
        return {str(int(word, 16)) for word in re.findall(r"0x[0-9a-f]+", identifiers)}

    def window_title(self, window):
        return self.xd("getwindowname", window)

    def focus(self, window):
        self.xd("windowactivate", "--sync", window)

    def close(self, window):
        self.focus(window)
        self.key("alt+F4")

    def key(self, *keys):
        for key in keys:
            if key == "F12":
                press_f12(self.display)
            else:
                self.xd("key", "--clearmodifiers", key)

    def set_clipboard(self, stack, text):
        if self.clipboard is not None:
            stop(self.clipboard)
        owner = start(
            stack, sys.executable, str(self.clipboard_script), text,
            env=self.environment, stdout=subprocess.PIPE, text=True,
        )
        if owner.stdout.readline().strip() != "READY":
            raise RuntimeError("Could not claim the GTK clipboard")
        self.clipboard = owner

    def picture(self, window):
        return decode(subprocess.check_output(
            ["import", "-window", str(window), "+repage", "png:-"], env=self.environment,
        ))

    def resize(self, window, width, height):
        self.xd("windowsize", "--sync", window, width * self.scale, height * self.scale)

    def move_pointer(self, window, x, y):
        self.xd("mousemove", "--window", window, x, y)

    def click(self, window, x, y):
        self.move_pointer(window, x, y)
        self.xd("click", 1)
