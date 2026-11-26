"""Persistent wlroots virtual keyboard backed by libxkbcommon US layout."""

from contextlib import contextmanager
import ctypes as ct
import ctypes.util
from dataclasses import dataclass
import os
import struct
import tempfile
import time


XKB_CONTEXT_NO_FLAGS = 0
XKB_KEYMAP_COMPILE_NO_FLAGS = 0
XKB_KEYMAP_FORMAT_TEXT_V1 = 1
XKB_KEYSYM_NO_FLAGS = 0
XKB_KEY_NO_SYMBOL = 0
XKB_MOD_INVALID = 0xffffffff
XKB_EVDEV_OFFSET = 8

WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1 = 1
WL_KEYBOARD_KEY_STATE_RELEASED = 0
WL_KEYBOARD_KEY_STATE_PRESSED = 1

MODIFIER_ALIASES = {
    "ctrl": "ctrl",
    "control": "ctrl",
    "shift": "shift",
    "alt": "alt",
    "mod1": "alt",
    "logo": "logo",
    "super": "logo",
    "win": "logo",
    "mod4": "logo",
}
MODIFIER_XKB_NAMES = {
    "shift": (b"Shift", "Shift_L"),
    "ctrl": (b"Control", "Control_L"),
    "alt": (b"Mod1", "Alt_L"),
    "logo": (b"Mod4", "Super_L"),
}
REQUIRED_MODIFIERS = ("shift", "ctrl", "alt")


class XkbRuleNames(ct.Structure):
    _fields_ = (
        ("rules", ct.c_char_p),
        ("model", ct.c_char_p),
        ("layout", ct.c_char_p),
        ("variant", ct.c_char_p),
        ("options", ct.c_char_p),
    )


@dataclass(frozen=True)
class KeyBinding:
    evdev_code: int
    shift: bool


def _load_xkbcommon():
    candidates = ["libxkbcommon.so.0"]
    found = ctypes.util.find_library("xkbcommon")
    if found and found not in candidates:
        candidates.append(found)
    candidates.append("libxkbcommon.so")
    for candidate in candidates:
        try:
            return ct.CDLL(candidate)
        except OSError:
            continue
    raise RuntimeError("Could not load libxkbcommon.so.0 for Wayland virtual keyboard")


def _configure_xkbcommon(lib):
    lib.xkb_context_new.argtypes = (ct.c_int,)
    lib.xkb_context_new.restype = ct.c_void_p
    lib.xkb_context_unref.argtypes = (ct.c_void_p,)
    lib.xkb_context_unref.restype = None

    lib.xkb_keymap_new_from_names.argtypes = (ct.c_void_p, ct.POINTER(XkbRuleNames), ct.c_int)
    lib.xkb_keymap_new_from_names.restype = ct.c_void_p
    lib.xkb_keymap_unref.argtypes = (ct.c_void_p,)
    lib.xkb_keymap_unref.restype = None
    lib.xkb_keymap_get_as_string.argtypes = (ct.c_void_p, ct.c_int)
    lib.xkb_keymap_get_as_string.restype = ct.c_void_p

    lib.xkb_keymap_min_keycode.argtypes = (ct.c_void_p,)
    lib.xkb_keymap_min_keycode.restype = ct.c_uint32
    lib.xkb_keymap_max_keycode.argtypes = (ct.c_void_p,)
    lib.xkb_keymap_max_keycode.restype = ct.c_uint32
    lib.xkb_keymap_num_levels_for_key.argtypes = (ct.c_void_p, ct.c_uint32, ct.c_uint32)
    lib.xkb_keymap_num_levels_for_key.restype = ct.c_uint32
    lib.xkb_keymap_key_get_syms_by_level.argtypes = (
        ct.c_void_p, ct.c_uint32, ct.c_uint32, ct.c_uint32, ct.POINTER(ct.POINTER(ct.c_uint32)),
    )
    lib.xkb_keymap_key_get_syms_by_level.restype = ct.c_int
    lib.xkb_keymap_mod_get_index.argtypes = (ct.c_void_p, ct.c_char_p)
    lib.xkb_keymap_mod_get_index.restype = ct.c_uint32

    lib.xkb_keysym_from_name.argtypes = (ct.c_char_p, ct.c_int)
    lib.xkb_keysym_from_name.restype = ct.c_uint32
    if hasattr(lib, "xkb_utf32_to_keysym"):
        lib.xkb_utf32_to_keysym.argtypes = (ct.c_uint32,)
        lib.xkb_utf32_to_keysym.restype = ct.c_uint32


def _free_c_string(pointer):
    libc = ct.CDLL(None)
    libc.free.argtypes = (ct.c_void_p,)
    libc.free.restype = None
    libc.free(pointer)


class XkbKeymap:
    """Compiled US XKB keymap with keysym-to-evdev and modifier tables."""

    def __init__(
        self,
        keymap_bytes,
        bindings,
        modifier_masks,
        modifier_codes,
        keysym_from_name,
        utf32_to_keysym=None,
    ):
        if not keymap_bytes.endswith(b"\0"):
            keymap_bytes += b"\0"
        self.serialized = keymap_bytes
        self._bindings = bindings
        self._modifier_masks = modifier_masks
        self._modifier_codes = modifier_codes
        self._keysym_from_name = keysym_from_name
        self._lookup_unicode = utf32_to_keysym

    @classmethod
    def load_us(cls, lib=None, free_string=_free_c_string):
        if lib is None:
            lib = _load_xkbcommon()
            _configure_xkbcommon(lib)

        context = lib.xkb_context_new(XKB_CONTEXT_NO_FLAGS)
        if not context:
            raise RuntimeError("Could not create libxkbcommon context")
        keymap = None
        try:
            names = XkbRuleNames(rules=b"evdev", model=b"pc105", layout=b"us", variant=b"", options=b"")
            keymap = lib.xkb_keymap_new_from_names(context, ct.byref(names), XKB_KEYMAP_COMPILE_NO_FLAGS)
            if not keymap:
                raise RuntimeError("Could not compile US XKB keymap via libxkbcommon")

            raw_ptr = lib.xkb_keymap_get_as_string(keymap, XKB_KEYMAP_FORMAT_TEXT_V1)
            if not raw_ptr:
                raise RuntimeError("Could not serialize US XKB keymap string")
            try:
                keymap_bytes = ct.string_at(raw_ptr) + b"\0"
            finally:
                if free_string is not None:
                    free_string(raw_ptr)

            bindings = {}
            min_kc = lib.xkb_keymap_min_keycode(keymap)
            max_kc = lib.xkb_keymap_max_keycode(keymap)
            syms_ptr = ct.POINTER(ct.c_uint32)()
            for level in (0, 1):
                for keycode in range(min_kc, max_kc + 1):
                    if keycode < XKB_EVDEV_OFFSET:
                        continue
                    if level >= lib.xkb_keymap_num_levels_for_key(keymap, keycode, 0):
                        continue
                    count = lib.xkb_keymap_key_get_syms_by_level(
                        keymap, keycode, 0, level, ct.byref(syms_ptr),
                    )
                    if count == 1:
                        sym = int(syms_ptr[0])
                        if sym != XKB_KEY_NO_SYMBOL and sym not in bindings:
                            bindings[sym] = KeyBinding(keycode - XKB_EVDEV_OFFSET, level == 1)

            def lookup_name(name):
                return int(lib.xkb_keysym_from_name(name.encode("utf-8"), XKB_KEYSYM_NO_FLAGS))

            lookup_utf32 = None
            if hasattr(lib, "xkb_utf32_to_keysym"):
                def lookup_utf32(codepoint):
                    return int(lib.xkb_utf32_to_keysym(codepoint))

            modifier_masks = {}
            modifier_codes = {}
            for canonical, (xkb_name, keysym_name) in MODIFIER_XKB_NAMES.items():
                mod_index = int(lib.xkb_keymap_mod_get_index(keymap, xkb_name))
                sym = lookup_name(keysym_name)
                binding = bindings.get(sym)
                if mod_index == XKB_MOD_INVALID or binding is None:
                    if canonical in REQUIRED_MODIFIERS:
                        raise RuntimeError(f"US XKB keymap is missing required modifier {canonical!r}")
                    continue
                modifier_masks[canonical] = 1 << mod_index
                modifier_codes[canonical] = binding.evdev_code

            return cls(
                keymap_bytes=keymap_bytes,
                bindings=bindings,
                modifier_masks=modifier_masks,
                modifier_codes=modifier_codes,
                keysym_from_name=lookup_name,
                utf32_to_keysym=lookup_utf32,
            )
        finally:
            if keymap:
                lib.xkb_keymap_unref(keymap)
            lib.xkb_context_unref(context)

    @staticmethod
    def canonical_modifier(name):
        canonical = MODIFIER_ALIASES.get(name.lower())
        if canonical is None:
            raise ValueError(f"Unsupported Wayland modifier: {name!r}")
        return canonical

    def modifier(self, name):
        canonical = self.canonical_modifier(name)
        if canonical not in self._modifier_masks or canonical not in self._modifier_codes:
            raise ValueError(f"Modifier {name!r} is not mapped in the US keymap")
        return canonical, self._modifier_codes[canonical], self._modifier_masks[canonical]

    def resolve(self, symbol):
        sym = self._keysym_from_name(symbol)
        if sym == XKB_KEY_NO_SYMBOL and len(symbol) == 1 and self._lookup_unicode is not None:
            sym = self._lookup_unicode(ord(symbol))
        if sym == XKB_KEY_NO_SYMBOL or sym not in self._bindings:
            raise ValueError(f"Key symbol {symbol!r} is not mapped in the US keymap")
        return self._bindings[sym]


@contextmanager
def _keymap_fd(keymap_bytes):
    if hasattr(os, "memfd_create"):
        fd = os.memfd_create("quickie-xkb-keymap")
        try:
            view = memoryview(keymap_bytes)
            while view:
                written = os.write(fd, view)
                view = view[written:]
            os.lseek(fd, 0, os.SEEK_SET)
            yield fd
        finally:
            os.close(fd)
    else:
        with tempfile.TemporaryFile() as handle:
            handle.write(keymap_bytes)
            handle.flush()
            handle.seek(0)
            yield handle.fileno()


class VirtualKeyboard:
    """Single persistent zwp_virtual_keyboard_v1 device for the isolated Wayland seat."""

    def __init__(self, connection, keyboard_id, keymap):
        self.connection = connection
        self.keyboard_id = keyboard_id
        self.keymap = keymap
        self._held_counts = {}
        self._mods_depressed = 0

    @classmethod
    def attach(cls, connection, seat_id, keymap=None):
        keymap = keymap or XkbKeymap.load_us()
        manager = connection.bind("zwp_virtual_keyboard_manager_v1")
        keyboard_id = connection.allocate_id()
        connection.request(manager, 0, struct.pack("=II", seat_id, keyboard_id))
        with _keymap_fd(keymap.serialized) as fd:
            connection.request(
                keyboard_id,
                0,  # zwp_virtual_keyboard_v1.keymap
                struct.pack("=II", WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, len(keymap.serialized)),
                fds=(fd,),
            )
        connection.roundtrip()
        return cls(connection, keyboard_id, keymap)

    def _timestamp_ms(self):
        return int(time.monotonic() * 1000) & 0xffffffff

    def _send_key(self, evdev_code, state):
        self.connection.request(
            self.keyboard_id,
            1,  # zwp_virtual_keyboard_v1.key
            struct.pack("=III", self._timestamp_ms(), evdev_code, state),
        )

    def _send_modifiers(self):
        self.connection.request(
            self.keyboard_id,
            2,  # zwp_virtual_keyboard_v1.modifiers
            struct.pack("=IIII", self._mods_depressed, 0, 0, 0),
        )

    def _hold_modifier(self, name):
        canonical, evdev_code, mask = self.keymap.modifier(name)
        count = self._held_counts.get(canonical, 0)
        self._held_counts[canonical] = count + 1
        if count == 0:
            self._send_key(evdev_code, WL_KEYBOARD_KEY_STATE_PRESSED)
            self._mods_depressed |= mask
            self._send_modifiers()

    def _release_modifier(self, name):
        canonical, evdev_code, mask = self.keymap.modifier(name)
        count = self._held_counts.get(canonical, 0)
        if count <= 0:
            return
        if count == 1:
            del self._held_counts[canonical]
            self._send_key(evdev_code, WL_KEYBOARD_KEY_STATE_RELEASED)
            self._mods_depressed &= ~mask
            self._send_modifiers()
        else:
            self._held_counts[canonical] = count - 1

    def hold_modifier(self, name):
        self._hold_modifier(name)
        self.connection.roundtrip()

    def release_modifier(self, name):
        self._release_modifier(name)
        self.connection.roundtrip()

    def tap(self, *keys):
        for chord in keys:
            *modifiers, symbol = chord.split("+")
            binding = self.keymap.resolve(symbol)
            extra_shift = binding.shift and not any(
                self.keymap.canonical_modifier(mod) == "shift" for mod in modifiers
            )
            held = [*modifiers, *(["shift"] if extra_shift else [])]
            pressed = []
            try:
                for modifier in held:
                    self._hold_modifier(modifier)
                    pressed.append(modifier)
                self._send_key(binding.evdev_code, WL_KEYBOARD_KEY_STATE_PRESSED)
                self._send_key(binding.evdev_code, WL_KEYBOARD_KEY_STATE_RELEASED)
            finally:
                for modifier in reversed(pressed):
                    self._release_modifier(modifier)
        self.connection.roundtrip()
