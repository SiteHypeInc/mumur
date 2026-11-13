"""Fake STT + LLM endpoints (OpenAI-compatible) that log what they receive."""
import json, os, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

LOG = sys.argv[2] if len(sys.argv) > 2 else "requests.log"
# Simulate a flaky provider: the first N transcription requests get a 503.
STT_FAILURES = [int(os.environ.get("MOCK_STT_FAILURES", "0"))]

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, obj):
        body = json.dumps(obj).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        self._send({"data": []})
    def do_POST(self):
        n = int(self.headers.get("content-length", 0)); raw = self.rfile.read(n)
        with open(LOG, "a") as f:
            if self.path.endswith("/audio/transcriptions"):
                failing = STT_FAILURES[0] > 0
                f.write(json.dumps({"path": self.path, "bytes": n, "has_wav": b"RIFF" in raw[:2000],
                                    "vocab": b"Vocabulary: GitHub" in raw, "failed": failing}) + "\n")
                if failing:
                    STT_FAILURES[0] -= 1
                    body = b'{"error":{"message":"service unavailable"}}'
                    self.send_response(503); self.send_header("content-length", str(len(body))); self.end_headers()
                    return self.wfile.write(body)
                return self._send({"text": "um so can you push it to git hub before friday"})
            req = json.loads(raw)
            f.write(json.dumps({"path": self.path, "exact_words": req["messages"][0]["content"].startswith("You add punctuation"),
                                "system_has_vocab": "GitHub" in req["messages"][0]["content"],
                                "user": req["messages"][1]["content"]}) + "\n")
            self._send({"choices": [{"message": {"role": "assistant", "content": "Can you push it to github before Friday?"}}]})

HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
