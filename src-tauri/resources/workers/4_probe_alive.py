#!/usr/bin/env python3
"""对分层后的 Web 候选做低速存活探测和高置信违规内容隔离。

脚本只读取既有 CSV，所有结果写入新目录。默认顺序为 P1 -> P2 -> P3 ->
Q2 -> Q3；仅弱指纹关联的 Q1 默认不主动访问。支持缓存、断点续跑、TLS
验证失败后的受控回退，以及 HTTP/HTTPS 协议回退。
"""

from __future__ import annotations

import argparse
import configparser
import csv
import gzip
import hashlib
import html
import http.client
import io
import ipaddress
import json
import os
import re
import socket
import sqlite3
import ssl
import sys
import threading
import time
import traceback
import zlib
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit, urlunsplit
from urllib.request import HTTPRedirectHandler, HTTPSHandler, Request, build_opener


BASE_DIR = Path(__file__).resolve().parent
DEFAULT_REFINED_DIR = BASE_DIR / "refined_output_20260714"
DEFAULT_OTHER_INPUT = BASE_DIR / "output" / "candidates.csv"
DEFAULT_OUTPUT_DIR = BASE_DIR / "probe_output_20260714"

HTTPS_PORTS = {443, 4443, 6443, 7443, 8443, 9443, 10443}
AMBIGUOUS_WEB_PROTOCOLS = {"", "tcp", "tls", "ssl", "unknown"}


def emit_progress(message: str, *, flush: bool = False) -> None:
    """Progress output must never abort a probe when the parent pipe closes."""
    try:
        print(message, flush=flush)
    except (BrokenPipeError, OSError, ValueError):
        pass
NAME_MARKERS = ("标题全称:", "正文全称:", "独有标题:", "独有正文:")
OUTCOMES = (
    "web_alive", "web_restricted", "browser_render_required",
    "virtual_host_required", "web_abnormal", "tcp_alive_non_http",
    "blocked_content", "unreachable", "skipped",
)
PROBE_FIELDS = [
    "probe_checked_at", "probe_input_url", "probe_effective_url",
    "probe_outcome", "probe_entry_state", "probe_status_code",
    "probe_title", "probe_content_type", "probe_server",
    "probe_latency_ms", "probe_tls_verified", "probe_attempts",
    "probe_body_truncated", "probe_error", "content_category",
    "content_risk_score", "content_matches", "body_sha256",
    "probe_body_bytes", "probe_virtual_host", "probe_redirect_scope",
]

GAMBLING_PATTERNS = [
    (r"在线赌博", 8), (r"网络赌博", 7), (r"赌博平台", 9),
    (r"博彩平台", 8), (r"在线博彩", 8), (r"真人视讯", 9),
    (r"真人娱乐", 7), (r"体育投注", 8), (r"彩票下注", 9),
    (r"在线娱乐城", 8), (r"澳门赌场", 8), (r"百家乐", 6),
    (r"送彩金", 5), (r"首存优惠", 6), (r"代理返佣", 5),
    (r"\bcasino\b", 7), (r"\bsportsbook\b", 8),
    (r"\bbetting\b", 6), (r"\bbaccarat\b", 7),
    (r"\broulette\b", 6), (r"\bslot\s*games?\b", 6),
]
PORN_PATTERNS = [
    (r"色情网站", 10), (r"成人网站", 10), (r"成人视频", 9),
    (r"成人影片", 9), (r"无码视频", 9), (r"无码中文字幕", 8),
    (r"激情视频", 8), (r"情色直播", 9), (r"约炮", 10),
    (r"裸聊", 9), (r"av女优", 8), (r"福利姬", 7),
    (r"\bporn(?:hub)?\b", 10), (r"\bporno\b", 10),
    (r"\bxxx\b", 7), (r"\bhentai\b", 8),
    (r"\bxvideos\b", 10), (r"\bxnxx\b", 10),
    (r"\badult\s*(?:video|movie|live|dating|content)s?\b", 8),
    (r"\bsex\s*(?:cam|video|movie|chat)s?\b", 9),
]
CUSTOM_PATTERNS: list[tuple[str, int]] = []
GAMBLING_NEGATIVE = (
    "打击赌博", "禁止赌博", "远离赌博", "赌博危害", "赌博治理",
    "反诈", "公安", "法院", "检察院", "举报", "专项整治", "普法",
    "风险监测", "新闻报道", "百科",
)
PORN_NEGATIVE = (
    "扫黄打非", "打击色情", "禁止色情", "举报色情", "色情治理",
    "公安", "法院", "检察院", "健康教育", "专项整治", "新闻报道", "百科",
)


def configure_content_rules(path: Path | None) -> str:
    """加载桌面端生成的关键词快照，并返回用于缓存隔离的规则摘要。"""
    if path is None:
        return "defaults"
    if not path.exists():
        raise ValueError(f"内容规则文件不存在：{path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"无法读取内容规则文件 {path}：{exc}") from exc
    if not isinstance(payload, dict):
        raise ValueError("内容规则必须是 JSON 对象")

    def keywords(name: str) -> list[str]:
        value = payload.get(name, [])
        if not isinstance(value, list):
            raise ValueError(f"内容规则 {name} 必须是数组")
        return list(dict.fromkeys(clean(item) for item in value if clean(item)))

    gambling = keywords("gamblingKeywords")
    porn = keywords("pornKeywords")
    negative = keywords("negativeKeywords")
    custom = keywords("customKeywords")
    replace_defaults = bool(payload.get("replaceDefaults", False))

    if replace_defaults:
        GAMBLING_PATTERNS[:] = []
        PORN_PATTERNS[:] = []
        globals()["GAMBLING_NEGATIVE"] = tuple()
        globals()["PORN_NEGATIVE"] = tuple()
    gambling_existing = {pattern for pattern, _ in GAMBLING_PATTERNS}
    porn_existing = {pattern for pattern, _ in PORN_PATTERNS}
    # Numeric-only fragments such as "91" occur in cache busters, dates and
    # ordinary paths.  Treating two appearances as adult content quarantined
    # hundreds of normal target URLs.  Numeric indicators require a descriptive
    # companion keyword and are therefore not valid standalone rules.
    porn = [item for item in porn if not item.isdecimal()]
    gambling = [item for item in gambling if not item.isdecimal()]
    GAMBLING_PATTERNS.extend(
        (escaped, 8) for item in gambling
        if (escaped := re.escape(item)) not in gambling_existing
    )
    PORN_PATTERNS.extend(
        (escaped, 9) for item in porn
        if (escaped := re.escape(item)) not in porn_existing
    )
    CUSTOM_PATTERNS[:] = [(re.escape(item), 12) for item in custom]
    if negative:
        globals()["GAMBLING_NEGATIVE"] = tuple(dict.fromkeys((*GAMBLING_NEGATIVE, *negative)))
        globals()["PORN_NEGATIVE"] = tuple(dict.fromkeys((*PORN_NEGATIVE, *negative)))

    canonical = json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def now_iso() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds")


def clean(value: Any) -> str:
    if value is None:
        return ""
    return re.sub(r"[\x00-\x08\x0B\x0C\x0E-\x1F]", "", str(value)).strip()


def parse_port(value: Any) -> int | None:
    try:
        port = int(float(clean(value)))
    except (TypeError, ValueError):
        return None
    return port if 1 <= port <= 65535 else None


def bracket_host(host: str) -> str:
    raw = host.strip("[]")
    try:
        return f"[{raw}]" if ipaddress.ip_address(raw).version == 6 else raw
    except ValueError:
        return raw.lower().rstrip(".")


def build_url(row: dict[str, str]) -> str | None:
    raw = clean(row.get("link")) or clean(row.get("host")) or clean(row.get("ip"))
    protocol = clean(row.get("protocol")).lower()
    port = parse_port(row.get("port"))
    if not raw:
        return None
    has_http = bool(re.match(r"^https?://", raw, flags=re.I))
    if "://" not in raw:
        try:
            if ipaddress.ip_address(raw).version == 6:
                raw = f"[{raw}]"
        except ValueError:
            pass
    try:
        parsed = urlsplit(raw if "://" in raw else f"//{raw}")
        host = parsed.hostname or ""
        parsed_port = parsed.port
    except ValueError:
        return None
    if not host:
        return None
    effective_port = parsed_port or port

    # 明确的 Web 资产使用原协议；任意自定义端口都会保留。
    is_web = has_http or "http" in protocol
    if not is_web:
        if not effective_port:
            return None
        netloc = f"{bracket_host(host)}:{effective_port}"
        # tls/ssl/tcp/未知协议可能是 FOFA 未识别出的 Web，后续会在任意端口
        # 上尝试 HTTP 和 HTTPS；明确的邮件、RDP 等协议只做 TCP 存活。
        scheme = "tcp+web" if protocol in AMBIGUOUS_WEB_PROTOCOLS or protocol in {"tls", "ssl"} else "tcp"
        if protocol in {"tls", "ssl"}:
            scheme = "tcp+tls"
        return urlunsplit((scheme, netloc, "", "", ""))

    if parsed.scheme.lower() in {"http", "https"}:
        scheme = parsed.scheme.lower()
    elif "https" in protocol or "ssl" in protocol or "tls" in protocol or port in HTTPS_PORTS:
        scheme = "https"
    else:
        scheme = "http"
    netloc = bracket_host(host)
    if effective_port and not ((scheme == "http" and effective_port == 80) or (scheme == "https" and effective_port == 443)):
        netloc = f"{netloc}:{effective_port}"
    path = re.sub(r"/{2,}", "/", parsed.path or "")
    return urlunsplit((scheme, netloc, path, "", ""))


def canonical_url(url: str) -> str:
    parsed = urlsplit(url)
    path = re.sub(r"/{2,}", "/", parsed.path or "")
    if path == "/":
        path = ""
    return urlunsplit((parsed.scheme.lower(), parsed.netloc.lower(), path.rstrip("/"), "", ""))


def row_identity(row: dict[str, str]) -> str:
    company = clean(row.get("company"))
    asset = clean(row.get("asset_key"))
    if asset:
        return f"{company}\x1f{asset}"
    return f"{company}\x1f{clean(row.get('link'))}\x1f{clean(row.get('host'))}\x1f{clean(row.get('ip'))}\x1f{clean(row.get('port'))}"


def other_bucket(row: dict[str, str]) -> str:
    company = clean(row.get("company"))
    cert_org = clean(row.get("cert.subject.org"))
    evidence = clean(row.get("evidence"))
    if company and company == cert_org:
        return "Q2"
    if any(marker in evidence for marker in NAME_MARKERS):
        return "Q3"
    return "Q1"


def is_non_public_literal(url: str) -> tuple[bool, str]:
    try:
        host = (urlsplit(url).hostname or "").lower().rstrip(".")
    except ValueError:
        return True, "invalid_host"
    if not host or host == "localhost" or host.endswith((".localhost", ".local", ".internal", ".lan", ".home", ".arpa")):
        return True, "local_hostname"
    try:
        address = ipaddress.ip_address(host)
    except ValueError:
        return False, ""
    return (not address.is_global), "non_public_ip" if not address.is_global else ""


def extract_title(text: str) -> str:
    match = re.search(r"<title\b[^>]*>(.*?)</title\s*>", text, flags=re.I | re.S)
    if not match:
        return ""
    title = re.sub(r"<[^>]+>", " ", match.group(1))
    return re.sub(r"\s+", " ", html.unescape(title)).strip()[:500]


def decode_body(data: bytes, content_type: str) -> str:
    charset_match = re.search(r"charset\s*=\s*[\"']?([^;\s\"']+)", content_type, flags=re.I)
    encodings = [charset_match.group(1)] if charset_match else []
    encodings.extend(["utf-8", "gb18030", "big5", "latin-1"])
    seen: set[str] = set()
    for encoding in encodings:
        if not encoding or encoding.lower() in seen:
            continue
        seen.add(encoding.lower())
        try:
            return data.decode(encoding)
        except (LookupError, UnicodeDecodeError):
            continue
    return data.decode("utf-8", errors="replace")


def decompress_body(data: bytes, encoding: str, max_bytes: int) -> bytes:
    try:
        if "gzip" in encoding.lower():
            with gzip.GzipFile(fileobj=io.BytesIO(data)) as stream:
                return stream.read(max_bytes)
        if "deflate" in encoding.lower():
            return zlib.decompressobj().decompress(data, max_bytes)
    except (OSError, EOFError, zlib.error):
        return data
    return data


def score_patterns(patterns: list[tuple[str, int]], negatives: tuple[str, ...], url: str, title: str, body: str) -> tuple[int, list[str]]:
    url_l = url.lower()
    title_l = title.lower()
    body_l = body.lower()
    score = 0
    matches: list[str] = []
    for pattern, weight in patterns:
        title_hits = len(re.findall(pattern, title_l, flags=re.I))
        url_hits = len(re.findall(pattern, url_l, flags=re.I))
        body_hits = len(re.findall(pattern, body_l, flags=re.I))
        if title_hits:
            score += weight * 2
            matches.append(f"title:{pattern}")
        if url_hits:
            score += weight * 2
            matches.append(f"url:{pattern}")
        if body_hits:
            score += weight * min(body_hits, 2)
            matches.append(f"body:{pattern}")
    for marker in negatives:
        if marker in title_l:
            score -= 12
            matches.append(f"negative-title:{marker}")
        elif marker in body_l:
            score -= 3
            matches.append(f"negative-body:{marker}")
    return max(score, 0), matches


def classify_content(url: str, title: str, body: str, threshold: int) -> tuple[str, int, str]:
    custom_score, custom_matches = score_patterns(CUSTOM_PATTERNS, tuple(), url, title, body)
    if custom_score >= threshold:
        return "custom_rule", custom_score, " | ".join(custom_matches[:20])
    gambling_score, gambling_matches = score_patterns(GAMBLING_PATTERNS, GAMBLING_NEGATIVE, url, title, body)
    porn_score, porn_matches = score_patterns(PORN_PATTERNS, PORN_NEGATIVE, url, title, body)
    if gambling_score >= porn_score and gambling_score >= threshold:
        return "gambling", gambling_score, " | ".join(gambling_matches[:20])
    if porn_score >= threshold:
        return "porn", porn_score, " | ".join(porn_matches[:20])
    score = max(gambling_score, porn_score)
    matches = gambling_matches if gambling_score >= porn_score else porn_matches
    return "clean", score, " | ".join(matches[:20])


def visible_text_length(body: str) -> int:
    text = re.sub(r"<(script|style)\b[^>]*>.*?</\1\s*>", " ", body, flags=re.I | re.S)
    text = re.sub(r"<[^>]+>", " ", text)
    return len(re.sub(r"\s+", "", html.unescape(text)))


def looks_like_javascript_shell(body: str, content_type: str) -> bool:
    if "html" not in content_type.lower() and "<html" not in body[:2000].lower():
        return False
    lower = body.lower()
    root = bool(re.search(r"id=[\"'](?:app|root|__next|__nuxt)[\"']", lower))
    scripts = len(re.findall(r"<script\b", lower))
    return visible_text_length(body) < 40 and scripts > 0 and (root or scripts >= 2)


def virtual_host_candidates(row: dict[str, str] | None) -> list[str]:
    if not row:
        return []
    result: list[str] = []
    for key in ("domain", "cname", "cname_domain", "cert.domain", "host", "link"):
        raw = clean(row.get(key))
        for item in re.split(r"[,;|\s]+", raw):
            item = item.strip().lstrip("*.")
            if "://" in item:
                try:
                    item = urlsplit(item).hostname or ""
                except ValueError:
                    item = ""
            item = item.rstrip(".").lower()
            try:
                ipaddress.ip_address(item)
                continue
            except ValueError:
                pass
            if "." in item and re.fullmatch(r"[a-z0-9._-]+", item) and item not in result:
                result.append(item)
    return result[:8]


def entry_state(status: int) -> str:
    if 200 <= status < 400 or status in {401, 403}:
        return "usable_or_restricted"
    if status in {404, 410}:
        return "reachable_but_path_missing"
    if 400 <= status < 500:
        return "reachable_client_error"
    if 500 <= status < 600:
        return "reachable_server_error"
    return "reachable_other"


class SafeRedirectHandler(HTTPRedirectHandler):
    max_redirections = 5

    def __init__(self, allow_private: bool):
        super().__init__()
        self.allow_private = allow_private

    def redirect_request(self, req: Request, fp: Any, code: int, msg: str, headers: Any, newurl: str) -> Request | None:
        if urlsplit(newurl).scheme.lower() not in {"http", "https"}:
            raise URLError(f"禁止跳转到非 HTTP(S) 协议：{newurl}")
        non_public, reason = is_non_public_literal(newurl)
        if non_public and not self.allow_private:
            raise URLError(f"禁止跳转到本地/私网地址：{reason}")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class RequestPacer:
    def __init__(self, rate: float, per_host_interval: float):
        self.interval = 1.0 / max(rate, 0.01)
        self.per_host_interval = max(per_host_interval, 0.0)
        self.lock = threading.Lock()
        self.next_global = 0.0
        self.next_host: dict[str, float] = {}

    def wait(self, host: str) -> None:
        with self.lock:
            now = time.monotonic()
            scheduled = max(now, self.next_global, self.next_host.get(host, 0.0))
            self.next_global = scheduled + self.interval
            self.next_host[host] = scheduled + self.per_host_interval
        delay = scheduled - now
        if delay > 0:
            time.sleep(delay)


def empty_result(url: str, outcome: str, error: str) -> dict[str, str]:
    result = {field: "" for field in PROBE_FIELDS}
    result.update({
        "probe_checked_at": now_iso(), "probe_input_url": url,
        "probe_effective_url": url, "probe_outcome": outcome,
        "probe_error": clean(error), "content_category": "not_checked",
        "content_risk_score": "0", "probe_attempts": "0",
    })
    return result


def fetch_once(url: str, timeout: float, max_body: int, verify_tls: bool, allow_private: bool, pacer: RequestPacer) -> tuple[dict[str, Any] | None, str]:
    parsed = urlsplit(url)
    pacer.wait(parsed.hostname or "")
    context = ssl.create_default_context() if verify_tls else ssl._create_unverified_context()
    opener = build_opener(SafeRedirectHandler(allow_private), HTTPSHandler(context=context))
    request = Request(url, headers={
        "User-Agent": "Mozilla/5.0 (compatible; AuthorizedAssetVerifier/1.0)",
        "Accept": "text/html,application/xhtml+xml,application/json,text/plain;q=0.8,*/*;q=0.2",
        "Accept-Encoding": "identity",
        "Range": f"bytes=0-{max_body - 1}",
        "Connection": "close",
    })
    started = time.monotonic()
    try:
        response = opener.open(request, timeout=timeout)
    except HTTPError as exc:
        response = exc
    except (URLError, TimeoutError, OSError, ssl.SSLError) as exc:
        return None, f"{type(exc).__name__}: {clean(exc)}"
    try:
        status = int(getattr(response, "status", None) or response.getcode() or 0)
        headers = response.headers
        raw = response.read(max_body + 1)
        truncated = len(raw) > max_body
        raw = raw[:max_body]
        raw = decompress_body(raw, clean(headers.get("Content-Encoding")), max_body)
        return {
            "status": status,
            "final_url": canonical_url(response.geturl()),
            "data": raw[:max_body],
            "truncated": truncated or len(raw) > max_body,
            "content_type": clean(headers.get("Content-Type")),
            "server": clean(headers.get("Server")),
            "latency_ms": int((time.monotonic() - started) * 1000),
            "tls_verified": "true" if parsed.scheme == "https" and verify_tls else ("false" if parsed.scheme == "https" else "not_applicable"),
        }, ""
    except (OSError, TimeoutError) as exc:
        return None, f"read {type(exc).__name__}: {clean(exc)}"
    finally:
        response.close()


def fetch_virtual_host_once(
    url: str, virtual_host: str, timeout: float, max_body: int, pacer: RequestPacer,
) -> tuple[dict[str, Any] | None, str]:
    """连接原 IP，但使用候选域名作为 HTTP Host 和 TLS SNI。"""
    parsed = urlsplit(url)
    connect_host = parsed.hostname or ""
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    if not connect_host or not virtual_host:
        return None, "virtual_host_missing_target"
    pacer.wait(f"{connect_host}|{virtual_host}")
    started = time.monotonic()
    sock: socket.socket | ssl.SSLSocket | None = None
    try:
        sock = socket.create_connection((connect_host, port), timeout=timeout)
        if parsed.scheme == "https":
            context = ssl._create_unverified_context()
            sock = context.wrap_socket(sock, server_hostname=virtual_host)
        path = parsed.path or "/"
        if parsed.query:
            path += f"?{parsed.query}"
        host_header = virtual_host
        if not ((parsed.scheme == "http" and port == 80) or (parsed.scheme == "https" and port == 443)):
            host_header = f"{virtual_host}:{port}"
        request = (
            f"GET {path} HTTP/1.1\r\nHost: {host_header}\r\n"
            "User-Agent: Mozilla/5.0 (compatible; AuthorizedAssetVerifier/1.0)\r\n"
            "Accept: text/html,application/xhtml+xml,application/json,text/plain;q=0.8,*/*;q=0.2\r\n"
            "Accept-Encoding: identity\r\nConnection: close\r\n\r\n"
        )
        sock.sendall(request.encode("ascii", errors="ignore"))
        response = http.client.HTTPResponse(sock)
        response.begin()
        raw = response.read(max_body + 1)
        truncated = len(raw) > max_body
        raw = decompress_body(raw[:max_body], clean(response.headers.get("Content-Encoding")), max_body)
        netloc = host_header
        return {
            "status": int(response.status or 0),
            "final_url": urlunsplit((parsed.scheme, netloc, parsed.path or "", parsed.query, "")),
            "data": raw[:max_body], "truncated": truncated,
            "content_type": clean(response.headers.get("Content-Type")),
            "server": clean(response.headers.get("Server")),
            "latency_ms": int((time.monotonic() - started) * 1000),
            "tls_verified": "false" if parsed.scheme == "https" else "not_applicable",
            "virtual_host": virtual_host,
        }, ""
    except (OSError, TimeoutError, ssl.SSLError, http.client.HTTPException) as exc:
        return None, f"virtual_host {type(exc).__name__}: {clean(exc)}"
    finally:
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass


def looks_like_tls_verification_error(error: str) -> bool:
    upper = error.upper()
    return "CERTIFICATE_VERIFY_FAILED" in upper or "CERTIFICATE VERIFY FAILED" in upper


def alternate_scheme(url: str) -> str:
    parsed = urlsplit(url)
    scheme = "http" if parsed.scheme == "https" else "https"
    return urlunsplit((scheme, parsed.netloc, parsed.path, "", ""))


def result_from_http(
    input_url: str, requested_url: str, fetched: dict[str, Any], args: argparse.Namespace,
    attempts: int, virtual_host: str = "",
) -> dict[str, str]:
    content_type = clean(fetched["content_type"])
    body = ""
    title = ""
    if any(token in content_type.lower() for token in ("text", "html", "json", "xml", "javascript")) or not content_type:
        body = decode_body(fetched["data"], content_type)
        title = extract_title(body)
    category, risk_score, matches = classify_content(fetched["final_url"], title, body, args.content_threshold)
    status = int(fetched["status"])
    state = entry_state(status)
    if category in {"gambling", "porn"}:
        outcome = "blocked_content"
    elif status in {401, 403, 407, 429}:
        outcome = "web_restricted"
    elif 200 <= status < 300 and len(fetched["data"]) == 0:
        outcome, state = "web_abnormal", "empty_response"
    elif 200 <= status < 300 and looks_like_javascript_shell(body, content_type):
        outcome, state = "browser_render_required", "javascript_shell"
    elif 200 <= status < 400:
        outcome = "web_alive"
    else:
        outcome = "web_abnormal"
    if virtual_host and outcome in {"web_alive", "web_restricted", "browser_render_required"}:
        outcome, state = "virtual_host_required", "virtual_host_required"
    requested_host = (urlsplit(requested_url).hostname or "").lower()
    final_host = (urlsplit(fetched["final_url"]).hostname or "").lower()
    redirect_scope = "external" if requested_host and final_host and requested_host != final_host else "same_host"
    return {
        "probe_checked_at": now_iso(), "probe_input_url": input_url,
        "probe_effective_url": clean(fetched["final_url"]), "probe_outcome": outcome,
        "probe_entry_state": state, "probe_status_code": str(status),
        "probe_title": clean(title), "probe_content_type": content_type,
        "probe_server": clean(fetched["server"]), "probe_latency_ms": str(fetched["latency_ms"]),
        "probe_tls_verified": clean(fetched["tls_verified"]), "probe_attempts": str(attempts),
        "probe_body_truncated": str(bool(fetched["truncated"])).lower(), "probe_error": "",
        "content_category": category, "content_risk_score": str(risk_score),
        "content_matches": clean(matches), "body_sha256": hashlib.sha256(fetched["data"]).hexdigest(),
        "probe_body_bytes": str(len(fetched["data"])), "probe_virtual_host": virtual_host,
        "probe_redirect_scope": redirect_scope,
    }


def probe_http_candidates(
    input_url: str, candidates: list[str], args: argparse.Namespace, pacer: RequestPacer,
    row: dict[str, str] | None = None,
) -> dict[str, str]:
    attempts = 0
    last_error = ""
    abnormal: dict[str, str] | None = None
    for candidate in candidates:
        for _ in range(args.retries + 1):
            attempts += 1
            fetched, error = fetch_once(candidate, args.timeout, args.max_body_bytes, True, args.allow_private, pacer)
            if fetched is None and candidate.startswith("https://") and not args.strict_tls and looks_like_tls_verification_error(error):
                attempts += 1
                fetched, error = fetch_once(candidate, args.timeout, args.max_body_bytes, False, args.allow_private, pacer)
            if fetched is not None:
                result = result_from_http(input_url, candidate, fetched, args, attempts)
                if result["probe_outcome"] != "web_abnormal":
                    return result
                abnormal = result
            last_error = error
    try:
        parsed_input = urlsplit(input_url)
        ipaddress.ip_address(parsed_input.hostname or "")
        is_ip_target = True
    except (ValueError, TypeError):
        is_ip_target = False
    if is_ip_target:
        for virtual_host in virtual_host_candidates(row):
            for candidate in candidates[:2]:
                attempts += 1
                fetched, error = fetch_virtual_host_once(candidate, virtual_host, args.timeout, args.max_body_bytes, pacer)
                if fetched is None:
                    last_error = error
                    continue
                result = result_from_http(input_url, candidate, fetched, args, attempts, virtual_host)
                if result["probe_outcome"] == "virtual_host_required":
                    return result
    if abnormal is not None:
        abnormal["probe_attempts"] = str(attempts)
        return abnormal
    result = empty_result(input_url, "unreachable", last_error or "no_http_response")
    result["probe_attempts"] = str(attempts)
    return result


def probe_tcp_only(url: str, timeout: float, pacer: RequestPacer, previous_attempts: int = 0, web_error: str = "") -> dict[str, str]:
    parsed = urlsplit(url)
    host = parsed.hostname or ""
    port = parsed.port
    if not host or not port:
        return empty_result(url, "skipped", "tcp_target_missing_host_or_port")
    started = time.monotonic()
    pacer.wait(host)
    try:
        with socket.create_connection((host, port), timeout=timeout):
            pass
    except (OSError, TimeoutError) as exc:
        result = empty_result(url, "unreachable", f"TCP {type(exc).__name__}: {clean(exc)}")
        result["probe_attempts"] = str(previous_attempts + 1)
        return result
    result = empty_result(url, "tcp_alive_non_http", "")
    result.update({
        "probe_effective_url": url,
        "probe_entry_state": "tcp_alive_non_http",
        "probe_latency_ms": str(int((time.monotonic() - started) * 1000)),
        "probe_attempts": str(previous_attempts + 1),
        "probe_tls_verified": "not_applicable",
        "probe_error": f"Web协议未识别，仅确认TCP存活：{clean(web_error)}" if web_error else "",
        "content_category": "not_applicable",
    })
    return result


def probe_url(url: str, args: argparse.Namespace, pacer: RequestPacer, row: dict[str, str] | None = None) -> dict[str, str]:
    try:
        parsed = urlsplit(url)
    except ValueError as exc:
        return empty_result(url, "skipped", f"invalid_url: {exc}")
    if parsed.scheme not in {"http", "https", "tcp", "tcp+web", "tcp+tls"} or not parsed.hostname:
        return empty_result(url, "skipped", "invalid_or_unsupported_target")
    non_public, reason = is_non_public_literal(url)
    if non_public and not args.allow_private:
        return empty_result(url, "skipped", reason)

    if parsed.scheme in {"tcp", "tcp+web", "tcp+tls"}:
        if parsed.scheme == "tcp":
            return probe_tcp_only(url, args.timeout, pacer)
        host = bracket_host(parsed.hostname or "")
        netloc = f"{host}:{parsed.port}"
        preferred = "https" if parsed.scheme == "tcp+tls" or parsed.port in HTTPS_PORTS else "http"
        secondary = "http" if preferred == "https" else "https"
        candidates = [f"{preferred}://{netloc}", f"{secondary}://{netloc}"]
        web_result = probe_http_candidates(url, candidates, args, pacer, row)
        if web_result.get("probe_outcome") != "unreachable":
            return web_result
        return probe_tcp_only(
            url, args.timeout, pacer,
            previous_attempts=int(web_result.get("probe_attempts") or 0),
            web_error=web_result.get("probe_error", ""),
        )

    candidates = [url]
    if args.scheme_fallback:
        candidates.append(alternate_scheme(url))
    web_result = probe_http_candidates(url, candidates, args, pacer, row)
    if web_result.get("probe_outcome") != "unreachable":
        return web_result
    host = bracket_host(parsed.hostname or "")
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    tcp_result = probe_tcp_only(
        f"tcp://{host}:{port}", args.timeout, pacer,
        previous_attempts=int(web_result.get("probe_attempts") or 0),
        web_error=web_result.get("probe_error", ""),
    )
    tcp_result["probe_input_url"] = url
    tcp_result["probe_effective_url"] = url
    return tcp_result


class ProbeCache:
    def __init__(self, path: Path, policy: str, max_age_hours: float):
        self.connection = sqlite3.connect(path)
        self.policy = policy
        self.max_age = max_age_hours * 3600
        self.connection.execute("PRAGMA journal_mode=WAL")
        self.connection.execute("PRAGMA synchronous=NORMAL")
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS probes (cache_key TEXT PRIMARY KEY, checked_at REAL NOT NULL, result_json TEXT NOT NULL)"
        )
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS kept_urls (effective_url TEXT PRIMARY KEY, first_stage TEXT NOT NULL, checked_at TEXT NOT NULL)"
        )
        self.connection.commit()

    def key(self, url: str) -> str:
        return hashlib.sha256(f"{self.policy}\x00{url}".encode()).hexdigest()

    def get(self, url: str) -> dict[str, str] | None:
        row = self.connection.execute("SELECT checked_at, result_json FROM probes WHERE cache_key=?", (self.key(url),)).fetchone()
        if not row or time.time() - float(row[0]) > self.max_age:
            return None
        return json.loads(row[1])

    def put(self, url: str, result: dict[str, str]) -> None:
        self.connection.execute(
            "INSERT OR REPLACE INTO probes(cache_key, checked_at, result_json) VALUES(?,?,?)",
            (self.key(url), time.time(), json.dumps(result, ensure_ascii=False)),
        )

    def keep(self, stage: str, result: dict[str, str]) -> None:
        if result.get("probe_outcome") not in {
            "web_alive", "web_restricted", "browser_render_required", "virtual_host_required",
        }:
            return
        effective = result.get("probe_effective_url") or result.get("probe_input_url")
        self.connection.execute(
            "INSERT OR IGNORE INTO kept_urls(effective_url, first_stage, checked_at) VALUES(?,?,?)",
            (effective, stage, result.get("probe_checked_at", now_iso())),
        )

    def commit(self) -> None:
        self.connection.commit()

    def export_kept_urls(self, path: Path) -> None:
        with path.open("w", encoding="utf-8") as target:
            for (url,) in self.connection.execute("SELECT effective_url FROM kept_urls ORDER BY effective_url"):
                target.write(f"{url}\n")

    def close(self) -> None:
        self.connection.commit()
        self.connection.close()


class BinaryCsvWriter:
    def __init__(self, path: Path, fieldnames: list[str], offset: int | None):
        self.path = path
        self.fieldnames = fieldnames
        if offset is None:
            self.file = path.open("w+b")
            self._write_values(fieldnames, bom=True)
        else:
            if not path.exists():
                raise RuntimeError(f"断点文件缺失：{path}")
            self.file = path.open("r+b")
            self.file.truncate(offset)
            self.file.seek(offset)
        self.buffer = io.StringIO(newline="")
        self.writer = csv.DictWriter(self.buffer, fieldnames=fieldnames, extrasaction="ignore")

    def _write_values(self, values: list[str], bom: bool = False) -> None:
        buffer = io.StringIO(newline="")
        csv.writer(buffer).writerow(values)
        encoding = "utf-8-sig" if bom else "utf-8"
        self.file.write(buffer.getvalue().encode(encoding))

    def writerow(self, row: dict[str, Any]) -> None:
        self.buffer.seek(0)
        self.buffer.truncate(0)
        self.writer.writerow({field: clean(row.get(field)) for field in self.fieldnames})
        self.file.write(self.buffer.getvalue().encode("utf-8"))

    def sync(self) -> int:
        self.file.flush()
        os.fsync(self.file.fileno())
        return self.file.tell()

    def close(self) -> None:
        self.file.close()


def atomic_write_json(path: Path, value: Any) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")
    os.replace(temporary, path)


def write_summary(output_dir: Path, state: dict[str, Any]) -> None:
    stages = state.get("stages", {})
    summary = {
        "updated_at": now_iso(),
        "output_dir": str(output_dir),
        "stage_order": state.get("stage_order", []),
        "stages": {
            name: {
                "complete": item.get("complete", False),
                "source_rows_scanned": item.get("next_data_row", 0),
                "selected_rows": item.get("selected_rows", 0),
                "counts": item.get("counts", {}),
            }
            for name, item in stages.items()
        },
        "notes": [
            "只有 web_alive/web_restricted/browser_render_required/virtual_host_required 属于浏览器资产。",
            "TCP 端口开放但没有 HTTP 响应时归入 tcp_alive_non_http，不再计入 Web 存活。",
            "404/5xx/空响应归入 web_abnormal；数据保留，但默认不进入 Web 人工队列。",
            "赌博/色情仅在规则达到高置信阈值时自动隔离，仍建议抽查 blocked_content。",
            "Q1 弱关联默认不探测，只有显式 --include-weak 且确认授权范围后才处理。",
        ],
    }
    atomic_write_json(output_dir / "summary.json", summary)
    with (output_dir / "summary.csv").open("w", encoding="utf-8-sig", newline="") as target:
        writer = csv.writer(target)
        writer.writerow(["stage", "complete", "source_rows_scanned", "selected_rows", *OUTCOMES])
        for name in state.get("stage_order", []):
            item = stages.get(name, {})
            counts = item.get("counts", {})
            writer.writerow([
                name, item.get("complete", False), item.get("next_data_row", 0),
                item.get("selected_rows", 0), *[counts.get(outcome, 0) for outcome in OUTCOMES],
            ])


def read_fieldnames(path: Path) -> list[str]:
    with path.open(encoding="utf-8-sig", newline="") as source:
        fields = list(csv.DictReader(source).fieldnames or [])
    if not fields:
        raise RuntimeError(f"CSV 没有表头：{path}")
    return fields


def source_signature(path: Path) -> dict[str, Any]:
    stat = path.stat()
    return {"path": str(path.resolve()), "size": stat.st_size, "mtime_ns": stat.st_mtime_ns}


def prepare_stage_writers(output_dir: Path, stage: str, fields: list[str], stage_state: dict[str, Any]) -> dict[str, BinaryCsvWriter]:
    result_fields = fields + [field for field in PROBE_FIELDS if field not in fields]
    offsets = stage_state.get("offsets", {})
    return {
        outcome: BinaryCsvWriter(output_dir / f"{stage}_{outcome}.csv", result_fields, offsets.get(outcome))
        for outcome in OUTCOMES
    }


def process_batch(
    stage: str,
    rows: list[tuple[dict[str, str], str]],
    writers: dict[str, BinaryCsvWriter],
    cache: ProbeCache,
    args: argparse.Namespace,
    pacer: RequestPacer,
) -> dict[str, int]:
    counts = {outcome: 0 for outcome in OUTCOMES}
    results: dict[str, dict[str, str]] = {}
    uncached: dict[str, tuple[dict[str, str], str]] = {}
    for row, url in rows:
        alias_signature = ",".join(virtual_host_candidates(row))
        cache_target = f"{url}\x1f{alias_signature}"
        cached = cache.get(cache_target)
        if cached is not None:
            results[url] = cached
        else:
            uncached.setdefault(url, (row, cache_target))

    executor = ThreadPoolExecutor(max_workers=args.workers)
    futures = {
        executor.submit(probe_url, url, args, pacer, row): (url, cache_target)
        for url, (row, cache_target) in uncached.items()
    }
    completed = 0
    progress_step = max(1, min(20, len(futures)))
    try:
        for future in as_completed(futures):
            url, cache_target = futures[future]
            try:
                result = future.result()
            except Exception as exc:  # 单个异常不能终止整批任务
                result = empty_result(url, "unreachable", f"worker {type(exc).__name__}: {exc}")
            results[url] = result
            cache.put(cache_target, result)
            completed += 1
            if completed % progress_step == 0 or completed == len(futures):
                emit_progress(f"      {stage} 当前批次：{completed}/{len(futures)} 个唯一目标完成", flush=True)
    except KeyboardInterrupt:
        for future in futures:
            future.cancel()
        executor.shutdown(wait=False, cancel_futures=True)
        raise
    else:
        executor.shutdown(wait=True)

    for row, url in rows:
        result = results[url]
        outcome = result.get("probe_outcome", "unreachable")
        if outcome not in OUTCOMES:
            outcome = "unreachable"
        writers[outcome].writerow({**row, **result})
        cache.keep(stage, result)
        counts[outcome] += 1
    cache.commit()
    return counts


def run_stage(
    stage: str,
    source_path: Path,
    selector: Callable[[dict[str, str]], bool],
    state: dict[str, Any],
    state_path: Path,
    output_dir: Path,
    cache: ProbeCache,
    args: argparse.Namespace,
    rate: float,
) -> None:
    signature = source_signature(source_path)
    fields = read_fieldnames(source_path)
    stage_state = state.setdefault("stages", {}).setdefault(stage, {
        "source": signature,
        "next_data_row": 0,
        "selected_rows": 0,
        "counts": {outcome: 0 for outcome in OUTCOMES},
        "offsets": {},
        "complete": False,
    })
    if stage_state.get("complete"):
        emit_progress(f"[=] {stage} 已完成，跳过")
        return
    if stage_state.get("source") != signature:
        raise RuntimeError(f"{stage} 的源文件在断点后发生变化，请指定新的 --output-dir：{source_path}")

    writers = prepare_stage_writers(output_dir, stage, fields, stage_state)
    pacer = RequestPacer(rate, args.per_host_interval)
    start_row = int(stage_state.get("next_data_row", 0))
    selected: list[tuple[dict[str, str], str]] = []
    current_row = 0
    emit_progress(f"[>] {stage} 开始/续跑：源={source_path.name}，起始行={start_row}，速率={rate:.2f} 请求/秒")

    def checkpoint(complete: bool = False) -> None:
        stage_state["next_data_row"] = current_row
        stage_state["offsets"] = {outcome: writer.sync() for outcome, writer in writers.items()}
        stage_state["complete"] = complete
        atomic_write_json(state_path, state)
        if complete:
            cache.export_kept_urls(output_dir / "alive_clean_urls.txt")
            cache.export_kept_urls(output_dir / "browser_accessible_urls.txt")
        write_summary(output_dir, state)

    try:
        with source_path.open(encoding="utf-8-sig", newline="") as source:
            reader = csv.DictReader(source)
            for _ in range(start_row):
                if next(reader, None) is None:
                    break
                current_row += 1
            for row in reader:
                current_row += 1
                if not selector(row):
                    if current_row % 5000 == 0 and not selected:
                        checkpoint(False)
                    continue
                url = build_url(row)
                if not url:
                    result = empty_result("", "skipped", "missing_or_non_web_url")
                    writers["skipped"].writerow({**row, **result})
                    stage_state["selected_rows"] += 1
                    stage_state["counts"]["skipped"] += 1
                else:
                    selected.append((row, canonical_url(url)))
                if len(selected) >= args.batch_size:
                    batch_counts = process_batch(stage, selected, writers, cache, args, pacer)
                    stage_state["selected_rows"] += len(selected)
                    for outcome, count in batch_counts.items():
                        stage_state["counts"][outcome] += count
                    selected.clear()
                    checkpoint(False)
                    counts_text = ", ".join(f"{key}={value}" for key, value in stage_state["counts"].items())
                    emit_progress(f"    {stage}: 已扫描 {current_row:,}，已选 {stage_state['selected_rows']:,}；{counts_text}")
            if selected:
                batch_counts = process_batch(stage, selected, writers, cache, args, pacer)
                stage_state["selected_rows"] += len(selected)
                for outcome, count in batch_counts.items():
                    stage_state["counts"][outcome] += count
                selected.clear()
            checkpoint(True)
    except KeyboardInterrupt:
        emit_progress(f"\n[!] 收到中断，{stage} 将从上一个完整批次续跑")
        raise
    finally:
        for writer in writers.values():
            writer.close()
    emit_progress(f"[+] {stage} 完成：{stage_state['counts']}")


def config_bool(section: configparser.SectionProxy | dict[str, str], key: str, fallback: bool) -> bool:
    value = section.get(key, str(fallback))
    return clean(value).lower() in {"1", "true", "yes", "on"}


def load_args() -> argparse.Namespace:
    pre = argparse.ArgumentParser(add_help=False)
    pre.add_argument("--config", type=Path, default=BASE_DIR / "config.ini")
    known, _ = pre.parse_known_args()
    config = configparser.ConfigParser(interpolation=None)
    if known.config.exists():
        config.read(known.config, encoding="utf-8")
    section: configparser.SectionProxy | dict[str, str] = config["probe"] if config.has_section("probe") else {}

    parser = argparse.ArgumentParser(description="分层探测 P1/P2/P3 及其他强关联网络资产，输出存活清单和隔离清单")
    parser.add_argument("--config", type=Path, default=known.config)
    parser.add_argument("--refined-dir", type=Path, default=BASE_DIR / section.get("refined_dir", str(DEFAULT_REFINED_DIR.relative_to(BASE_DIR))))
    parser.add_argument("--other-input", type=Path, default=BASE_DIR / section.get("other_input", str(DEFAULT_OTHER_INPUT.relative_to(BASE_DIR))))
    parser.add_argument("--output-dir", type=Path, default=BASE_DIR / section.get("output_dir", str(DEFAULT_OUTPUT_DIR.relative_to(BASE_DIR))))
    parser.add_argument("--priority-rate", type=float, default=float(section.get("priority_rate", "20.0")))
    parser.add_argument("--other-rate", type=float, default=float(section.get("other_rate", "10.0")))
    parser.add_argument("--per-host-interval", type=float, default=float(section.get("per_host_interval", "1.5")))
    parser.add_argument("--workers", type=int, default=int(section.get("workers", "64")))
    parser.add_argument("--timeout", type=float, default=float(section.get("timeout", "6")))
    parser.add_argument("--retries", type=int, default=int(section.get("retries", "0")))
    parser.add_argument("--max-body-bytes", type=int, default=int(section.get("max_body_bytes", "524288")))
    parser.add_argument("--batch-size", type=int, default=int(section.get("batch_size", "200")))
    parser.add_argument("--cache-hours", type=float, default=float(section.get("cache_hours", "24")))
    parser.add_argument("--content-threshold", type=int, default=int(section.get("content_threshold", "12")))
    parser.add_argument("--content-rules", type=Path, help="桌面端生成的内容分类规则 JSON 快照")
    parser.add_argument("--include-other", action=argparse.BooleanOptionalAction, default=config_bool(section, "include_other", True))
    parser.add_argument("--include-weak", action=argparse.BooleanOptionalAction, default=config_bool(section, "include_weak", False))
    parser.add_argument("--allow-private", action=argparse.BooleanOptionalAction, default=config_bool(section, "allow_private", False))
    parser.add_argument("--strict-tls", action=argparse.BooleanOptionalAction, default=config_bool(section, "strict_tls", False))
    parser.add_argument("--scheme-fallback", action=argparse.BooleanOptionalAction, default=config_bool(section, "scheme_fallback", True))
    args = parser.parse_args()
    if args.priority_rate <= 0 or args.other_rate <= 0 or args.workers < 1 or args.timeout <= 0:
        parser.error("速率、workers 和 timeout 必须大于 0")
    if args.batch_size < 1 or args.max_body_bytes < 4096 or args.retries < 0:
        parser.error("batch-size >= 1、max-body-bytes >= 4096、retries >= 0")
    return args


def main() -> int:
    args = load_args()
    try:
        rules_hash = configure_content_rules(args.content_rules)
    except ValueError as exc:
        emit_progress(f"[-] {exc}")
        return 2
    try:
        csv.field_size_limit(sys.maxsize)
    except OverflowError:
        csv.field_size_limit(2**31 - 1)

    priority_sources = [
        ("P1", args.refined_dir / "P1_active_strong.csv"),
        ("P2", args.refined_dir / "P2_strong_needs_validation.csv"),
        ("P3", args.refined_dir / "P3_name_candidates.csv"),
    ]
    missing = [str(path) for _, path in priority_sources if not path.exists()]
    if args.include_other and not args.other_input.exists():
        missing.append(str(args.other_input))
    if missing:
        emit_progress("[-] 缺少输入文件：\n  " + "\n  ".join(missing))
        return 2

    args.output_dir.mkdir(parents=True, exist_ok=True)
    state_path = args.output_dir / "checkpoint.json"
    if state_path.exists():
        state = json.loads(state_path.read_text(encoding="utf-8"))
        if int(state.get("probe_version", 0)) != 3:
            emit_progress("[-] 旧版探测断点使用 alive_clean 混合口径，不能继续复用；请新建一次‘复测现有资产’任务")
            return 2
        emit_progress(f"[=] 发现断点，继续输出目录：{args.output_dir}")
    else:
        existing = [path for path in args.output_dir.iterdir()]
        if existing:
            emit_progress(f"[-] 输出目录已有内容但没有 checkpoint.json，请改用新的 --output-dir：{args.output_dir}")
            return 2
        stage_order = ["P1", "P2", "P3"]
        if args.include_other:
            stage_order.extend(["Q2", "Q3"])
        if args.include_weak:
            stage_order.append("Q1")
        state = {"version": 1, "probe_version": 3, "created_at": now_iso(), "stage_order": stage_order, "stages": {}}
        atomic_write_json(state_path, state)

    # 允许先用 --no-include-other 完成 P1/P2/P3，之后在同一输出目录追加
    # Q2/Q3；显式启用弱关联时再追加 Q1，不影响已经完成的阶段。
    desired_order = ["P1", "P2", "P3"]
    if args.include_other:
        desired_order.extend(["Q2", "Q3"])
    if args.include_weak:
        desired_order.append("Q1")
    stage_order = state.setdefault("stage_order", [])
    for stage in desired_order:
        if stage not in stage_order:
            stage_order.append(stage)
    atomic_write_json(state_path, state)

    priority_ids: set[str] = set()
    for _, path in priority_sources:
        with path.open(encoding="utf-8-sig", newline="") as source:
            priority_ids.update(row_identity(row) for row in csv.DictReader(source))

    policy = json.dumps({
        "probe_version": 3,
        "threshold": args.content_threshold, "max_body": args.max_body_bytes,
        "allow_private": args.allow_private, "strict_tls": args.strict_tls,
        "scheme_fallback": args.scheme_fallback, "retries": args.retries,
        "content_rules": rules_hash,
    }, sort_keys=True)
    cache = ProbeCache(args.output_dir / "probe_cache.sqlite3", policy, args.cache_hours)
    try:
        for stage, path in priority_sources:
            run_stage(stage, path, lambda row: True, state, state_path, args.output_dir, cache, args, args.priority_rate)
        if args.include_other:
            for stage in ("Q2", "Q3"):
                run_stage(
                    stage, args.other_input,
                    lambda row, expected=stage: row_identity(row) not in priority_ids and other_bucket(row) == expected,
                    state, state_path, args.output_dir, cache, args, args.other_rate,
                )
        if args.include_weak:
            run_stage(
                "Q1", args.other_input,
                lambda row: row_identity(row) not in priority_ids and other_bucket(row) == "Q1",
                state, state_path, args.output_dir, cache, args, args.other_rate,
            )
    except KeyboardInterrupt:
        write_summary(args.output_dir, state)
        emit_progress(f"[=] 已保存断点。重新执行同一命令即可续跑：{state_path}")
        return 130
    except Exception as exc:
        error_text = f"{type(exc).__name__}: {exc}"
        try:
            (args.output_dir / "probe_error.log").write_text(
                traceback.format_exc(), encoding="utf-8"
            )
        except OSError:
            pass
        emit_progress(f"[-] {error_text}")
        return 1
    finally:
        cache.export_kept_urls(args.output_dir / "alive_clean_urls.txt")
        cache.export_kept_urls(args.output_dir / "browser_accessible_urls.txt")
        cache.close()

    write_summary(args.output_dir, state)
    emit_progress(f"[+] 全部指定阶段完成。新结果目录：{args.output_dir}")
    emit_progress(f"[+] 去重后的浏览器可访问 URL：{args.output_dir / 'browser_accessible_urls.txt'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
