"""Serve real attachments, an interrupted response, and controllable active transfers."""

from collections import Counter
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Event, Lock, Thread

PAYLOAD = b"The WebKit download wrote these exact fixture bytes.\n"
CHUNK = b"x" * 65536
SHUTDOWN_PAYLOAD = CHUNK + PAYLOAD
FIXTURE = Path(__file__).with_name("fixture.html").read_bytes()


class Fixture(BaseHTTPRequestHandler):
    def do_GET(self):
        with self.server.count_lock:
            self.server.counts[self.path] += 1
            attempt = self.server.counts[self.path]
        if self.path == "/":
            self.respond("text/html; charset=utf-8", FIXTURE)
        elif self.path == "/?throwaway":
            body = FIXTURE.replace(b'href="/saved"', b'href="/private"').replace(b"#1c3956", b"#27382a")
            self.headers_for("text/html; charset=utf-8", len(body), None, "profile=throwaway")
            self.wfile.write(body)
        elif self.path == "/private" and self.headers.get("Cookie") == "profile=throwaway":
            if attempt == 1:
                self.stream("throwaway-only.txt", "private", 16 * len(CHUNK))
            else:
                self.respond("application/octet-stream", PAYLOAD, "throwaway-only.txt")
        elif self.path == "/saved":
            self.respond("text/plain", PAYLOAD, "network-proof.txt")
        elif self.path == "/named":
            self.respond("text/plain", PAYLOAD)
        elif self.path == "/automatic":
            self.respond("application/octet-stream", PAYLOAD, "automatic.txt")
        elif self.path == "/retry" and attempt == 1:
            self.stream("quarterly report.txt", "retry", 16 * len(CHUNK))
        elif self.path == "/retry":
            self.respond("application/octet-stream", PAYLOAD, "quarterly report.txt")
        elif self.path == "/cancel" and attempt == 1:
            self.stream("dataset.zip", "cancel", 16 * len(CHUNK))
        elif self.path == "/cancel":
            self.respond("application/octet-stream", PAYLOAD, "dataset.zip")
        elif self.path == "/known":
            self.stream("archive.bin", "known", 16 * len(CHUNK))
        elif self.path == "/unknown" and attempt == 1:
            self.stream("stream.log", "unknown")
        elif self.path == "/unknown":
            self.stream("shutdown-completed.txt", "shutdown", len(SHUTDOWN_PAYLOAD))
        else:
            self.send_error(404)

    def respond(self, media, data, filename=None):
        self.headers_for(media, len(data), filename)
        self.wfile.write(data)

    def stream(self, filename, key, size=None):
        self.headers_for("application/octet-stream", size, filename)
        self.wfile.write(CHUNK)
        self.wfile.flush()
        self.server.release[key].wait(40)
        if key == "known":
            self.wfile.write(CHUNK * 15)
        elif key == "shutdown":
            self.wfile.write(PAYLOAD)

    def headers_for(self, media, size, filename, cookie=None):
        self.send_response(200)
        self.send_header("Content-Type", media)
        self.send_header("Cache-Control", "no-store")
        if size is not None:
            self.send_header("Content-Length", str(size))
        if filename is not None:
            self.send_header("Content-Disposition", f'attachment; filename="{filename}"')
        if cookie is not None:
            self.send_header("Set-Cookie", cookie)
        self.end_headers()

    def log_message(self, *_args):
        pass


@contextmanager
def endpoint():
    with ThreadingHTTPServer(("127.0.0.1", 0), Fixture) as server:
        server.count_lock = Lock()
        server.counts = Counter()
        server.release = {
            key: Event() for key in ("retry", "cancel", "known", "unknown", "private", "shutdown")
        }
        worker = Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            yield server
        finally:
            for release in server.release.values():
                release.set()
            server.shutdown()
            worker.join()
