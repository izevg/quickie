"""Keep a virtual pointer attached to the isolated Wayland seat for Sway IPC clicks."""

import socket
import struct


DISPLAY = 1
REGISTRY = 2
SEAT = 4
POINTER_MANAGER = 5
POINTER = 6


def attach_pointer(stack, display):
    # These protocol requests only use uints and strings. The file descriptor is owned
    # by the desktop scope so the seat retains its pointer capability for every story.
    connection = stack.enter_context(socket.socket(socket.AF_UNIX))
    connection.settimeout(5)
    connection.connect(str(display))
    events = stack.enter_context(connection.makefile("rb"))

    def request(object_id, opcode, payload):
        connection.sendall(struct.pack("=II", object_id, ((8 + len(payload)) << 16) | opcode) + payload)

    def roundtrip(callback):
        request(DISPLAY, 0, struct.pack("=I", callback))  # wl_display.sync
        globals_by_interface = {}
        while header := events.read(8):
            sender, opcode_and_size = struct.unpack("=II", header)
            opcode = opcode_and_size & 0xffff
            payload = events.read((opcode_and_size >> 16) - 8)
            if sender == callback:
                return globals_by_interface
            if sender == DISPLAY and opcode == 0:
                message = payload[12:].split(b"\0", 1)[0].decode()
                raise RuntimeError(f"Wayland virtual pointer connection failed: {message}")
            if sender == REGISTRY and opcode == 0:  # wl_registry.global
                name, length = struct.unpack("=II", payload[:8])
                interface = payload[8:7 + length].decode()
                globals_by_interface[interface] = name
        raise RuntimeError("Sway disconnected before the virtual pointer was ready")

    request(DISPLAY, 1, struct.pack("=I", REGISTRY))  # wl_display.get_registry
    globals_by_interface = roundtrip(3)

    def bind(interface, object_id):
        if interface not in globals_by_interface:
            raise RuntimeError(f"Sway did not advertise {interface}")
        name = globals_by_interface[interface]
        encoded = interface.encode() + b"\0"
        padding = b"\0" * (-len(encoded) % 4)
        request(REGISTRY, 0, struct.pack("=II", name, len(encoded)) + encoded + padding
                + struct.pack("=II", 1, object_id))  # wl_registry.bind

    bind("wl_seat", SEAT)
    bind("zwlr_virtual_pointer_manager_v1", POINTER_MANAGER)
    request(POINTER_MANAGER, 0, struct.pack("=II", SEAT, POINTER))  # create_virtual_pointer
    roundtrip(7)
