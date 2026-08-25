#!/usr/bin/env python3
"""Bounded, dependency-free SRC helpers mounted read-only into Strix jobs.

The adapter does not discover targets. Every operation requires one explicit URL
or request contract from Oviraptor's evidence packet and emits compact JSON.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import http.client
import json
import socket
import ssl
import sys
import time
import urllib.parse
from pathlib import Path

MAX_REQUEST_BYTES = 64 * 1024
MAX_RESPONSE_BYTES = 256 * 1024
SAFE_METHODS = {"GET", "HEAD", "OPTIONS"}
DENIED_METHODS = {"DELETE", "CONNECT", "TRACE"}


def fail(message: str, code: int = 2) -> None:
    print(json.dumps({"ok": False, "error": message}, ensure_ascii=False))
    raise SystemExit(code)


def explicit_url(value: str) -> urllib.parse.SplitResult:
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        fail("url must use http:// or https:// with an explicit host")
    return parsed


def bounded_timeout(value: float) -> float:
    return max(0.5, min(float(value), 15.0))


def recv_bounded(sock: socket.socket) -> bytes:
    chunks: list[bytes] = []
    size = 0
    while size < MAX_RESPONSE_BYTES:
        try:
            chunk = sock.recv(min(16384, MAX_RESPONSE_BYTES - size))
        except socket.timeout:
            break
        if not chunk:
            break
        chunks.append(chunk)
        size += len(chunk)
    return b"".join(chunks)


def raw_http(args: argparse.Namespace) -> None:
    parsed = explicit_url(args.url)
    request = Path(args.request_file).read_bytes()
    if not request or len(request) > MAX_REQUEST_BYTES:
        fail(f"raw request must be 1..{MAX_REQUEST_BYTES} bytes")
    first_line = request.splitlines()[0].decode("latin-1", "replace")
    parts = first_line.split()
    if len(parts) < 3:
        fail("raw request is missing a valid request line")
    method = parts[0].upper()
    if method in DENIED_METHODS:
        fail(f"method {method} is disabled by the built-in adapter")
    host = parsed.hostname or ""
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    header_host = ""
    for line in request.splitlines()[1:]:
        decoded = line.decode("latin-1", "replace")
        if decoded.lower().startswith("host:"):
            header_host = decoded.split(":", 1)[1].strip().split(":", 1)[0].strip("[]").lower()
            break
    if header_host and header_host != host.lower():
        fail("raw request Host must match the explicit --url host")
    timeout = bounded_timeout(args.timeout)
    started = time.monotonic()
    sock = socket.create_connection((host, port), timeout=timeout)
    try:
        sock.settimeout(timeout)
        if parsed.scheme == "https":
            context = ssl.create_default_context()
            sock = context.wrap_socket(sock, server_hostname=host)
            sock.settimeout(timeout)
        sock.sendall(request)
        response = recv_bounded(sock)
    finally:
        sock.close()
    status_line = response.splitlines()[0].decode("latin-1", "replace") if response else ""
    print(json.dumps({
        "ok": True,
        "adapter": "bounded-raw-http",
        "target": f"{parsed.scheme}://{host}:{port}",
        "requestMethod": method,
        "requestBytes": len(request),
        "responseBytes": len(response),
        "responseSha256": hashlib.sha256(response).hexdigest(),
        "statusLine": status_line[:240],
        "elapsedMs": round((time.monotonic() - started) * 1000),
        "truncated": len(response) >= MAX_RESPONSE_BYTES,
    }, ensure_ascii=False))


def load_contract(path: str) -> dict:
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail("request contract must be a JSON object")
    explicit_url(str(value.get("url", "")))
    method = str(value.get("method", "GET")).upper()
    if method in DENIED_METHODS:
        fail(f"method {method} is disabled")
    if method not in SAFE_METHODS:
        cleanup = value.get("cleanup")
        invariant = str(value.get("invariant", "")).strip()
        if not isinstance(cleanup, dict) or not cleanup.get("url") or not invariant:
            fail("write race contracts require cleanup.url and a business invariant")
        explicit_url(str(cleanup["url"]))
        cleanup_method = str(cleanup.get("method", "POST")).upper()
        if cleanup_method in DENIED_METHODS:
            fail(f"cleanup method {cleanup_method} is disabled")
    return value


def request_once(contract: dict, timeout: float) -> dict:
    parsed = explicit_url(str(contract["url"]))
    method = str(contract.get("method", "GET")).upper()
    headers = contract.get("headers") if isinstance(contract.get("headers"), dict) else {}
    body = contract.get("body", "")
    if isinstance(body, (dict, list)):
        body = json.dumps(body, ensure_ascii=False, separators=(",", ":"))
    body_bytes = str(body).encode("utf-8")
    if len(body_bytes) > MAX_REQUEST_BYTES:
        fail("request body exceeds adapter limit")
    connection_type = http.client.HTTPSConnection if parsed.scheme == "https" else http.client.HTTPConnection
    connection = connection_type(parsed.hostname, parsed.port, timeout=timeout)
    path = urllib.parse.urlunsplit(("", "", parsed.path or "/", parsed.query, ""))
    started = time.monotonic()
    try:
        connection.request(method, path, body=body_bytes or None, headers={str(k): str(v) for k, v in headers.items()})
        response = connection.getresponse()
        data = response.read(MAX_RESPONSE_BYTES)
        return {
            "status": response.status,
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
            "elapsedMs": round((time.monotonic() - started) * 1000),
            "contentType": response.getheader("content-type", "")[:160],
        }
    except Exception as error:  # compact per-attempt evidence, never a traceback
        return {"error": f"{type(error).__name__}: {error}"[:300]}
    finally:
        connection.close()


def cleanup_contract(contract: dict, timeout: float) -> dict | None:
    cleanup = contract.get("cleanup")
    if not isinstance(cleanup, dict) or not cleanup.get("url"):
        return None
    cleanup = dict(cleanup)
    cleanup.setdefault("method", "POST")
    return request_once(cleanup, timeout)


def race(args: argparse.Namespace) -> None:
    contract = load_contract(args.contract)
    concurrency = max(2, min(int(args.concurrency), 64))
    attempts = max(concurrency, min(int(args.attempts), 128))
    timeout = bounded_timeout(args.timeout)
    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as executor:
        results = list(executor.map(lambda _: request_once(contract, timeout), range(attempts)))
    cleanup = cleanup_contract(contract, timeout)
    statuses: dict[str, int] = {}
    hashes: dict[str, int] = {}
    errors = 0
    for result in results:
        if "error" in result:
            errors += 1
            continue
        status = str(result.get("status"))
        digest = str(result.get("sha256"))
        statuses[status] = statuses.get(status, 0) + 1
        hashes[digest] = hashes.get(digest, 0) + 1
    print(json.dumps({
        "ok": True,
        "adapter": "bounded-race-scheduler",
        "attempts": attempts,
        "concurrency": concurrency,
        "statuses": statuses,
        "distinctResponseHashes": len(hashes),
        "errors": errors,
        "cleanup": cleanup,
        "invariant": str(contract.get("invariant", ""))[:500],
        "elapsedMs": round((time.monotonic() - started) * 1000),
        "note": "response differences are evidence candidates; the business invariant still decides whether a race exists",
    }, ensure_ascii=False))


def main() -> None:
    parser = argparse.ArgumentParser(description="Oviraptor bounded SRC assurance adapter")
    sub = parser.add_subparsers(dest="command", required=True)
    raw = sub.add_parser("raw-http")
    raw.add_argument("--url", required=True)
    raw.add_argument("--request-file", required=True)
    raw.add_argument("--timeout", type=float, default=5)
    raw.set_defaults(handler=raw_http)
    race_parser = sub.add_parser("race")
    race_parser.add_argument("--contract", required=True)
    race_parser.add_argument("--concurrency", type=int, default=8)
    race_parser.add_argument("--attempts", type=int, default=16)
    race_parser.add_argument("--timeout", type=float, default=5)
    race_parser.set_defaults(handler=race)
    args = parser.parse_args()
    args.handler(args)


if __name__ == "__main__":
    main()
