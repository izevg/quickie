"""A loopback chat endpoint that records page data and confirms cancelled transfers."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from threading import Event, Lock, Thread


class Endpoint:
    def __init__(self, stack):
        self.server = stack.enter_context(ThreadingHTTPServer(("127.0.0.1", 0), Handler))
        self.lock = Lock()
        self.requests = []
        self.probes = []
        self.stopping = Event()
        self.disconnected = Event()
        self.server.endpoint = self
        worker = Thread(target=self.server.serve_forever, daemon=True)
        worker.start()
        stack.callback(worker.join)
        stack.callback(self.server.shutdown)

    @property
    def address(self):
        return f"http://127.0.0.1:{self.server.server_port}/v1"

    def add(self, request):
        with self.lock:
            self.requests.append(request)

    def snapshot(self):
        with self.lock:
            return self.requests.copy()

    def reject_probe(self, path):
        with self.lock:
            self.probes.append(path)

    def probe_paths(self):
        with self.lock:
            return self.probes.copy()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.endpoint.add({"unexpected_get": self.path})
        self.reply(405, b"", "application/json")

    def do_POST(self):
        assert self.path in {"/v1/chat/completions", "/v1/responses"}, self.path
        assert self.headers.get("Authorization") is None
        assert self.headers.get("Cookie") is None
        size = int(self.headers["Content-Length"])
        assert 0 < size <= 100_000
        request = json.loads(self.rfile.read(size))
        if self.path == "/v1/responses":
            self.server.endpoint.reject_probe(self.path)
            self.reply(404, b"404 page not found", "text/plain; charset=utf-8")
            return
        if "tools" in request:
            self.server.endpoint.reject_probe(self.path)
            # Ollama 0.12.11 wraps the scheduler's capability failure in an OpenAI error.
            error = {"type": "api_error", "code": None, "param": None,
                     "message": "registry.ollama.ai/library/observatory-fixture:latest does not support tools"}
            self.reply(400, json.dumps({"error": error}).encode(), "application/json")
            return
        assert request["stream"] is True, "the compatible Chat request still streams"
        self.server.endpoint.add(request)
        prompt = request["messages"][-1]["content"]
        if "Return a service failure" in prompt:
            self.reply(503, b'{"error":{"message":"Observatory endpoint unavailable"}}', "application/json")
        elif "Keep this answer open" in prompt:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", "4096")
            self.end_headers()
            self.wfile.write(chunk("An unfinished answer"))
            self.wfile.flush()
            self.server.endpoint.stopping.set()
            self.connection.settimeout(6)
            if self.connection.recv(1) == b"":
                self.server.endpoint.disconnected.set()
        elif "Repeat the opening time" in prompt:
            self.reply(200, chunk("A follow-up answer: dusk.") + b"data: [DONE]\n\n", "text/event-stream")
        elif "Read the replacement observatory" in prompt:
            body = chunk("The southern observatory opens at sunrise.")
            self.reply(200, body + b"data: [DONE]\n\n", "text/event-stream")
        else:
            body = chunk("The opening time is dusk. [[cite: The northern observatory opens at dusk.]]")
            self.reply(200, body + b"data: [DONE]\n\n", "text/event-stream")

    def reply(self, status, body, mime):
        self.send_response(status)
        self.send_header("Content-Type", mime)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


def chunk(text):
    body = json.dumps({"choices": [{"delta": {"content": text}}]})
    return f"data: {body}\n\n".encode()
