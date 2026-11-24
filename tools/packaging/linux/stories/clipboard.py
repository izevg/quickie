"""Hold a GTK clipboard selection so a story can paste exact Unicode text."""

import ctypes
import sys


gtk = ctypes.CDLL("libgtk-4.so.1")
glib = ctypes.CDLL("libglib-2.0.so.0")
gtk.gtk_init.argtypes = []
gtk.gtk_init.restype = None
gtk.gdk_display_get_default.argtypes = []
gtk.gdk_display_get_default.restype = ctypes.c_void_p
gtk.gdk_display_get_clipboard.argtypes = [ctypes.c_void_p]
gtk.gdk_display_get_clipboard.restype = ctypes.c_void_p
gtk.gdk_clipboard_set_text.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
gtk.gdk_clipboard_set_text.restype = None
glib.g_main_loop_new.argtypes = [ctypes.c_void_p, ctypes.c_int]
glib.g_main_loop_new.restype = ctypes.c_void_p
glib.g_main_loop_run.argtypes = [ctypes.c_void_p]
glib.g_main_loop_run.restype = None

gtk.gtk_init()
display = gtk.gdk_display_get_default()
if not display:
    raise RuntimeError("No GTK display for the story clipboard")
clipboard = gtk.gdk_display_get_clipboard(display)
gtk.gdk_clipboard_set_text(clipboard, sys.argv[1].encode())
print("READY", flush=True)
glib.g_main_loop_run(glib.g_main_loop_new(None, 0))
