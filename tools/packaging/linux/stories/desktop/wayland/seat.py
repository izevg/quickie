"""Manage the isolated Wayland seat connection and virtual pointer."""

import array
import socket
import struct


DISPLAY = 1
REGISTRY = 2


class WaylandConnection:
    """Minimal synchronous Wayland client over the isolated Sway Unix socket."""

    def __init__(self, stack, display):
        self._socket = stack.enter_context(socket.socket(socket.AF_UNIX))
        self._socket.settimeout(5)
        self._socket.connect(str(display))
        self._events = stack.enter_context(self._socket.makefile("rb"))
        self._next_id = REGISTRY + 1
        self._globals = {}
        self.request(DISPLAY, 1, struct.pack("=I", REGISTRY))  # wl_display.get_registry
        self._globals = self.roundtrip()

    def allocate_id(self):
        object_id = self._next_id
        self._next_id += 1
        return object_id

    def request(self, object_id, opcode, payload=b"", fds=()):
        header = struct.pack("=II", object_id, ((8 + len(payload)) << 16) | opcode)
        message = header + payload
        if not fds:
            self._socket.sendall(message)
            return
        ancillary = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", fds))]
        sent = self._socket.sendmsg([message], ancillary)
        if sent < len(message):
            self._socket.sendall(message[sent:])

    def roundtrip(self):
        callback = self.allocate_id()
        self.request(DISPLAY, 0, struct.pack("=I", callback))  # wl_display.sync
        globals_by_interface = {}
        while True:
            header = self._events.read(8)
            if len(header) < 8:
                raise RuntimeError("Sway disconnected before the Wayland roundtrip completed")
            sender, opcode_and_size = struct.unpack("=II", header)
            opcode = opcode_and_size & 0xffff
            payload_size = (opcode_and_size >> 16) - 8
            payload = self._events.read(payload_size) if payload_size else b""
            if len(payload) < payload_size:
                raise RuntimeError("Sway disconnected mid-event during Wayland roundtrip")
            if sender == callback:
                return globals_by_interface
            if sender == DISPLAY and opcode == 0:
                message = payload[12:].split(b"\0", 1)[0].decode("utf-8", errors="replace")
                raise RuntimeError(f"Wayland seat connection failed: {message}")
            if sender == REGISTRY and opcode == 0:  # wl_registry.global
                name, length = struct.unpack("=II", payload[:8])
                interface = payload[8:7 + length].decode()
                globals_by_interface[interface] = name

    def bind(self, interface, version=1):
        if interface not in self._globals:
            raise RuntimeError(f"Sway did not advertise {interface}")
        object_id = self.allocate_id()
        name = self._globals[interface]
        encoded = interface.encode() + b"\0"
        padding = b"\0" * (-len(encoded) % 4)
        self.request(
            REGISTRY,
            0,  # wl_registry.bind
            struct.pack("=II", name, len(encoded)) + encoded + padding
            + struct.pack("=II", version, object_id),
        )
        return object_id


def attach_pointer(connection, seat_id):
    """Attach a persistent virtual pointer so Sway IPC clicks keep a pointer capability."""
    pointer_manager = connection.bind("zwlr_virtual_pointer_manager_v1")
    pointer = connection.allocate_id()
    connection.request(pointer_manager, 0, struct.pack("=II", seat_id, pointer))
    connection.roundtrip()
    return pointer
