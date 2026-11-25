"""A loopback provider with a rejected Chat route and typed Responses completions."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from threading import Event, Lock, Thread


FIRST = "Read the northern field guide"
FOLLOW_UP = "Recall the guide's opening time"
FORBIDDEN = "Try the unavailable browser action"
AUTH_FAILURE = "Check the provider authorization error"
HELD = "Read the source when this response is released"
NEW_ORIGIN = "Answer using the new endpoint"

READ_REASONING = {
    "type": "reasoning", "id": "rs_fixture_read", "summary": [],
    "encrypted_content": "sealed-fixture-read-context",
}
READ_CALL = {
    "type": "function_call", "id": "fc_fixture_read", "call_id": "call-fixture-source",
    "name": "read_page", "arguments": '{"page":"source"}', "status": "completed",
}
ANSWER_REASONING = {
    "type": "reasoning", "id": "rs_fixture_final", "summary": [],
    "encrypted_content": "sealed-fixture-answer-context",
}
ANSWER = {
    "type": "message", "id": "msg_fixture_final", "role": "assistant", "status": "completed",
    "phase": "final_answer", "content": [{
        "type": "output_text", "text": "It opens at dusk. [[cite: The northern observatory opens at dusk.]]",
        "annotations": [],
    }],
}


class Endpoint:
    def __init__(self, stack):
        self.server = stack.enter_context(ThreadingHTTPServer(("127.0.0.1", 0), Handler))
        self.server.endpoint = self
        self.release = Event()
        self.released_response = Event()
        self.lock = Lock()
        self.requests = []
        worker = Thread(target=self.server.serve_forever, daemon=True)
        worker.start()
        stack.callback(worker.join)
        stack.callback(self.server.shutdown)
        stack.callback(self.release.set)

    @property
    def address(self):
        return f"http://127.0.0.1:{self.server.server_port}/v1"

    def record(self, path, request):
        with self.lock:
            self.requests.append((path, request))

    def snapshot(self):
        with self.lock:
            return self.requests.copy()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.endpoint.record(self.path, None)
        self.reply(405, {"error": {"message": "This fixture accepts POST"}})

    def do_POST(self):
        assert self.headers.get("Authorization") is None
        assert self.headers.get("Cookie") is None
        count = int(self.headers["Content-Length"])
        assert 0 < count < 100_000
        body = json.loads(self.rfile.read(count))
        endpoint = self.server.endpoint
        endpoint.record(self.path, body)

        if self.path != "/v1/responses":
            self.reply(404, {"error": {"code": "route_not_found", "message": "Route unavailable"}})
            return

        inputs = body["input"]
        prompt = next(item["content"] for item in reversed(inputs) if item.get("role") == "user")
        if prompt == FIRST:
            tool_result = any(item.get("type") == "function_call_output" for item in inputs)
            if tool_result:
                self.completed(ANSWER_REASONING, ANSWER)
            else:
                self.completed(READ_REASONING, READ_CALL)
        elif prompt == FOLLOW_UP:
            self.text("It still opens at dusk.")
        elif prompt == FORBIDDEN:
            self.completed({
                "type": "function_call", "id": "fc_fixture_navigation",
                "call_id": "call-fixture-navigation", "status": "completed",
                "name": "open_link", "arguments": '{"id":1}',
            })
        elif prompt == AUTH_FAILURE:
            self.reply(401, {"error": {"code": "invalid_api_key", "message": "Authorization refused"}})
        elif prompt == HELD:
            if endpoint.release.wait(12):
                try:
                    self.completed(READ_CALL | {"id": "fc_fixture_held", "call_id": "call-fixture-held"})
                except (BrokenPipeError, ConnectionResetError):
                    # The browser may have cancelled the request before the model returned.
                    pass
                finally:
                    endpoint.released_response.set()
        elif prompt == NEW_ORIGIN:
            self.text("Answered by the newly configured endpoint.")
        else:
            self.reply(400, {"error": {"message": "Unexpected fixture prompt"}})

    def completed(self, *output):
        self.reply(200, {"object": "response", "status": "completed", "output": output})

    def text(self, answer):
        self.completed({
            "type": "message", "role": "assistant", "status": "completed", "phase": "final_answer",
            "content": [{"type": "output_text", "text": answer, "annotations": []}],
        })

    def reply(self, status, value):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_args):
        pass
