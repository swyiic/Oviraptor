import json
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ADAPTER = Path(__file__).resolve().parents[1] / "10_src_assurance_adapter.py"


class QuietHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b'{"ok":true}'
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


class SrcAssuranceAdapterTests(unittest.TestCase):
    def test_raw_http_is_bounded_and_returns_structured_evidence(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        port = listener.getsockname()[1]

        def serve():
            connection, _ = listener.accept()
            connection.recv(65536)
            connection.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
            connection.close()
            listener.close()

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        with tempfile.TemporaryDirectory() as root:
            request_file = Path(root) / "request.txt"
            request_file.write_bytes(
                f"GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n".encode()
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(ADAPTER),
                    "raw-http",
                    "--url",
                    f"http://127.0.0.1:{port}/health",
                    "--request-file",
                    str(request_file),
                ],
                capture_output=True,
                text=True,
                check=True,
                timeout=10,
            )
        payload = json.loads(result.stdout)
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["statusLine"], "HTTP/1.1 200 OK")
        self.assertEqual(payload["requestMethod"], "GET")

    def test_race_scheduler_caps_parallel_requests_and_summarizes_results(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), QuietHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as root:
                contract = Path(root) / "contract.json"
                contract.write_text(
                    json.dumps(
                        {
                            "method": "GET",
                            "url": f"http://127.0.0.1:{server.server_port}/object/1",
                        }
                    ),
                    encoding="utf-8",
                )
                result = subprocess.run(
                    [
                        sys.executable,
                        str(ADAPTER),
                        "race",
                        "--contract",
                        str(contract),
                        "--concurrency",
                        "2",
                        "--attempts",
                        "4",
                    ],
                    capture_output=True,
                    text=True,
                    check=True,
                    timeout=15,
                )
            payload = json.loads(result.stdout)
            self.assertTrue(payload["ok"])
            self.assertEqual(payload["attempts"], 4)
            self.assertEqual(payload["statuses"], {"200": 4})
            self.assertEqual(payload["errors"], 0)
        finally:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
