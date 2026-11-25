"""Send plain F12 without xdotool's XKB modifier synthesis."""

import ctypes as ct


def press_f12(display_name):
    x11 = ct.CDLL("libX11.so.6")
    xtest = ct.CDLL("libXtst.so.6")
    x11.XOpenDisplay.argtypes = (ct.c_char_p,)
    x11.XOpenDisplay.restype = ct.c_void_p
    x11.XStringToKeysym.argtypes = (ct.c_char_p,)
    x11.XStringToKeysym.restype = ct.c_ulong
    x11.XKeysymToKeycode.argtypes = (ct.c_void_p, ct.c_ulong)
    x11.XKeysymToKeycode.restype = ct.c_ubyte
    x11.XSync.argtypes = (ct.c_void_p, ct.c_int)
    x11.XCloseDisplay.argtypes = (ct.c_void_p,)
    xtest.XTestFakeKeyEvent.argtypes = (ct.c_void_p, ct.c_uint, ct.c_int, ct.c_ulong)
    xtest.XTestFakeKeyEvent.restype = ct.c_int

    display = x11.XOpenDisplay(display_name.encode())
    if not display:
        raise RuntimeError("Could not connect to the isolated X11 display")
    try:
        keycode = x11.XKeysymToKeycode(display, x11.XStringToKeysym(b"F12"))
        if not keycode:
            raise RuntimeError("The isolated X11 display has no F12 keycode")
        if not xtest.XTestFakeKeyEvent(display, keycode, 1, 0):
            raise RuntimeError("Could not press F12 on the isolated X11 display")
        if not xtest.XTestFakeKeyEvent(display, keycode, 0, 0):
            raise RuntimeError("Could not release F12 on the isolated X11 display")
        x11.XSync(display, 0)
    finally:
        x11.XCloseDisplay(display)
