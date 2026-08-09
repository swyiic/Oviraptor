#!/usr/bin/env python3
"""Oviraptor deterministic frontend reconnaissance.

This worker only reads the supplied web targets and their referenced JavaScript.
It produces structured inventory for Sentinel; it does not run exploit PoCs.
Potential secrets are always masked and hashed before they leave this process.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import ipaddress
import json
import re
import shutil
import ssl
import subprocess
import time
from datetime import datetime, timezone
from html.parser import HTMLParser
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qsl, unquote, urljoin, urlparse
from urllib.request import Request, urlopen


USER_AGENT = "oviraptor-Sentinel/0.5 (+authorized-security-inventory)"
API_HINT = re.compile(r"(?:^|/)(?:api|rest|graphql|oauth|auth|login)(?:/|$)", re.I)
QUOTED_PATH = re.compile(r"[\"'`]((?:https?://|/)[^\"'`\s<>]{2,300})[\"'`]")
ROUTE_PATH = re.compile(r"(?:\bpath\s*:\s*|<Route[^>]+\bpath\s*=\s*)[\"'`]([^\"'`]{1,240})[\"'`]", re.I)
BASE_URL = re.compile(r"\b(?:baseURL|apiBaseUrl|API_BASE_URL|VITE_[A-Z0-9_]*URL|REACT_APP_[A-Z0-9_]*URL)\s*[:=]\s*[\"'`]([^\"'`]+)[\"'`]", re.I)
FETCH_CALL = re.compile(r"\bfetch\s*\(\s*[\"'`]([^\"'`]+)[\"'`]", re.I)


def node_compatible_path(value: str) -> str:
    """Node CLI does not reliably resolve Windows extended-length paths."""
    if not value.startswith("\\\\?\\"):
        return value
    if value.startswith("\\\\?\\UNC\\"):
        return "\\\\" + value[8:]
    return value[4:]
AXIOS_CALL = re.compile(r"\b(?:axios\.)?(get|post|put|patch|delete|head|options)\s*\(\s*[\"'`]([^\"'`]+)[\"'`]", re.I)
AJAX_URL = re.compile(r"\burl\s*:\s*[\"'`]([^\"'`]+)[\"'`]", re.I)
OBJECT_KEYS = re.compile(r"\b([A-Za-z_$][\w$]{1,80})\s*:")
DYNAMIC_SCRIPT = re.compile(
    r"\b(?:import|require)\s*\(\s*[\"'`]([^\"'`\s]{1,300}\.m?js(?:\?[^\"'`\s]*)?)[\"'`]",
    re.I,
)
RELATIVE_SCRIPT = re.compile(
    r"[\"'`]((?:\.{0,2}/)[^\"'`\s<>]{1,300}\.m?js(?:\?[^\"'`\s<>]*)?)[\"'`]",
    re.I,
)
ANY_SCRIPT_STRING = re.compile(
    r"[\"'`]((?!data:|javascript:)[^\"'`\s<>]{1,500}\.(?:m?js|jsx|tsx?)(?:\?[^\"'`\s<>]*)?)[\"'`]",
    re.I,
)
SEMVER = r"(\d+\.\d+(?:\.\d+)?(?:[-+][0-9A-Za-z.-]+)?)"
REGISTRATION_CORE = re.compile(
    r"(?:^|[/_.?=&-])(?:register|registration|signup|sign-up|sign_up|"
    r"create-account|create_account|account-create|join-now|new-account|"
    r"registrarse|registro|inscription|registrieren|registrazione|cadastro|registratie|"
    r"регистрация|kayıt-ol|daftar|dang-ky|สมัครสมาชิก|rejestracja|"
    r"users?[/_.-](?:register|signup|create)|members?[/_.-](?:register|signup|create)|"
    r"(?:auth|oauth|identity|passport|sso)[/_.-](?:register|registration|signup))(?:$|[/_.?=&])",
    re.I,
)
REGISTRATION_SUPPORT = re.compile(
    r"(?:send|request|issue|verify|validate|check)[/_.-]?"
    r"(?:sms|email|mobile|phone|otp|captcha|verification)?[/_.-]?"
    r"(?:code|otp|captcha|availability|exists)|"
    r"(?:invite|invitation|referral)[/_.-]?(?:code|validate|verify|accept)|"
    r"(?:username|email|mobile|phone)[/_.-]?(?:available|availability|exists|check)",
    re.I,
)
REGISTRATION_LABELS = re.compile(
    r"(?:\b(?:register|registration|sign\s*up|create\s+(?:an?\s+)?account|join\s+now|"
    r"new\s+account|enroll)\b|注册(?:账号|账户|用户)?|创建(?:账号|账户)|新用户注册|"
    r"新規登録|アカウント作成|회원가입|가입하기|registrarse|registro|inscripción|"
    r"inscription|créer\s+un\s+compte|registrieren|konto\s+erstellen|registrazione|"
    r"crea\s+un\s+account|cadastro|criar\s+conta|регистрац(?:ия|ии)|создать\s+аккаунт|"
    r"تسجيل|إنشاء\s+حساب|kayıt\s+ol|daftar|đăng\s+ký|สมัครสมาชิก|rejestracja|registratie)",
    re.I,
)
RUNTIME_SIGNAL_PATTERNS = [
    ("browser_storage", "Browser storage/cookie", re.compile(r"\b(?:localStorage|sessionStorage|document\.cookie)\b", re.I)),
    ("route_runtime", "Client-side router", re.compile(r"\b(?:vue-router|react-router|createRouter|useNavigate|router\.push)\b", re.I)),
    ("network_runtime", "Computed network request", re.compile(r"\b(?:fetch\s*\(|XMLHttpRequest|axios\.(?:request|get|post|put|patch|delete)\s*\()", re.I)),
    ("anti_debug", "Anti-debug behavior", re.compile(r"\bdebugger\b|Function\.prototype\.constructor|console\.clear\s*\(", re.I)),
]

CRYPTO_SIGNAL_PATTERNS = [
    ("encoding", "Base64", "encode/decode", "high", re.compile(r"\b(?:atob|btoa)\s*\(|\bCryptoJS\.enc\.Base64\b|\bBase64\.(?:encode|decode)\s*\(", re.I)),
    ("hash", "MD5", "hash", "high", re.compile(r"\b(?:CryptoJS\.)?MD5\s*\(|\bcreateHash\s*\(\s*['\"]md5['\"]", re.I)),
    ("hash", "SHA-1", "hash", "high", re.compile(r"\b(?:CryptoJS\.)?SHA1\s*\(|\b(?:digest|createHash)\s*\(\s*['\"]sha-?1['\"]", re.I)),
    ("hash", "SHA-256", "hash", "high", re.compile(r"\b(?:CryptoJS\.)?SHA256\s*\(|\b(?:digest|createHash)\s*\(\s*['\"]sha-?256['\"]", re.I)),
    ("hash", "SHA-512", "hash", "high", re.compile(r"\b(?:CryptoJS\.)?SHA512\s*\(|\b(?:digest|createHash)\s*\(\s*['\"]sha-?512['\"]", re.I)),
    ("hash", "HMAC", "sign/verify", "high", re.compile(r"\bCryptoJS\.Hmac(?:MD5|SHA1|SHA256|SHA512)\s*\(|\bcreateHmac\s*\(", re.I)),
    ("hash", "PBKDF2", "derive", "high", re.compile(r"\b(?:CryptoJS\.)?PBKDF2\s*\(|\bderiveKey\s*\(", re.I)),
    ("hash", "bcrypt", "hash/verify", "medium", re.compile(r"\bbcrypt(?:js)?\.(?:hash|compare)\s*\(", re.I)),
    ("hash", "scrypt", "derive", "medium", re.compile(r"\bscrypt(?:Sync)?\s*\(", re.I)),
    ("hash", "Argon2", "hash/verify", "medium", re.compile(r"\bargon2\.(?:hash|verify)\s*\(", re.I)),
    ("symmetric", "AES", "encrypt/decrypt", "high", re.compile(r"\bCryptoJS\.AES\.(?:encrypt|decrypt)\s*\(|\bcreateCipheriv\s*\(\s*['\"]aes-", re.I)),
    ("symmetric", "DES", "encrypt/decrypt", "high", re.compile(r"\bCryptoJS\.DES\.(?:encrypt|decrypt)\s*\(|\bcreateCipheriv\s*\(\s*['\"]des(?:-|['\"])", re.I)),
    ("symmetric", "3DES", "encrypt/decrypt", "high", re.compile(r"\bCryptoJS\.TripleDES\.(?:encrypt|decrypt)\s*\(|\bcreateCipheriv\s*\(\s*['\"](?:des-ede3|3des)", re.I)),
    ("symmetric", "ChaCha20", "encrypt/decrypt", "medium", re.compile(r"\b(?:chacha20|ChaCha20Poly1305)\b", re.I)),
    ("asymmetric", "RSA", "encrypt/decrypt/sign", "high", re.compile(r"\bJSEncrypt\b|\b(?:publicEncrypt|privateDecrypt|privateEncrypt|publicDecrypt)\s*\(|\bRSA-OAEP\b", re.I)),
    ("china", "SM2", "encrypt/decrypt/sign", "high", re.compile(r"\bsm2\.(?:doEncrypt|doDecrypt|doSignature|doVerifySignature)\s*\(", re.I)),
    ("china", "SM3", "hash", "high", re.compile(r"\bsm3\s*\(|\bsm3\.(?:update|digest)\s*\(", re.I)),
    ("china", "SM4", "encrypt/decrypt", "high", re.compile(r"\bsm4\.(?:encrypt|decrypt)\s*\(", re.I)),
]


class PageParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.scripts: list[str] = []
        self.links: list[str] = []
        self.link_records: list[dict[str, str]] = []
        self.meta: dict[str, str] = {}
        self.angular_version = ""
        self.inline_scripts: list[str] = []
        self.forms: list[dict[str, str]] = []
        self._script_buffer: list[str] | None = None
        self._link_stack: list[dict[str, Any]] = []
        self._form_stack: list[dict[str, Any]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = {str(k).lower(): str(v or "") for k, v in attrs}
        lowered_tag = tag.lower()
        if lowered_tag == "script":
            if values.get("src"):
                self.scripts.append(values["src"])
            else:
                self._script_buffer = []
        rel_tokens = set(values.get("rel", "").lower().split())
        if lowered_tag == "link" and values.get("href") and rel_tokens.intersection({
            "modulepreload", "preload", "prefetch"
        }):
            if values.get("as", "").lower() in {"", "script"} or values.get("href", "").lower().endswith(".js"):
                self.scripts.append(values["href"])
        if lowered_tag == "a" and values.get("href"):
            self.links.append(values["href"])
            self._link_stack.append({"url": values["href"], "textParts": []})
        if lowered_tag == "form":
            form: dict[str, Any] = {
                "action": values.get("action", ""),
                "method": values.get("method", "GET").upper(),
                "id": values.get("id", ""),
                "name": values.get("name", ""),
                "class": values.get("class", ""),
                "text": "",
                "_textParts": [],
            }
            self.forms.append(form)
            self._form_stack.append(form)
        if lowered_tag == "meta":
            key = values.get("name") or values.get("property") or values.get("http-equiv")
            if key:
                self.meta[key] = values.get("content", "")
        if values.get("ng-version"):
            self.angular_version = values["ng-version"]

    def handle_data(self, data: str) -> None:
        if self._script_buffer is not None:
            self._script_buffer.append(data)
            return
        for link in self._link_stack:
            link["textParts"].append(data)
        for form in self._form_stack:
            form["_textParts"].append(data)

    def handle_endtag(self, tag: str) -> None:
        lowered_tag = tag.lower()
        if lowered_tag == "script" and self._script_buffer is not None:
            body = "".join(self._script_buffer).strip()
            if body:
                self.inline_scripts.append(body)
            self._script_buffer = None
        elif lowered_tag == "a" and self._link_stack:
            link = self._link_stack.pop()
            self.link_records.append({
                "url": str(link["url"]),
                "text": re.sub(r"\s+", " ", "".join(link["textParts"])).strip(),
            })
        elif lowered_tag == "form" and self._form_stack:
            form = self._form_stack.pop()
            form["text"] = re.sub(r"\s+", " ", "".join(form.pop("_textParts", []))).strip()


def unique(items: list[Any], key=lambda value: json.dumps(value, sort_keys=True, ensure_ascii=False)) -> list[Any]:
    result, seen = [], set()
    for item in items:
        marker = key(item)
        if marker not in seen:
            seen.add(marker)
            result.append(item)
    return result


class ReconCache:
    """Reuse immutable script bodies and AST output across URLs in one batch."""

    def __init__(self) -> None:
        self.responses: dict[str, tuple[str, dict[str, str], int, str]] = {}
        self.ast: dict[str, dict[str, Any]] = {}


def cached_fetch(cache: ReconCache, url: str, timeout: float, max_bytes: int) -> tuple[str, dict[str, str], int, str]:
    cached = cache.responses.get(url)
    if cached is not None and (
        max_bytes <= 0 or len(cached[0].encode("utf-8", "ignore")) <= max_bytes
    ):
        return cached
    response = fetch(url, timeout, max_bytes)
    if response[2] < 400:
        cache.responses[url] = response
    return response


def run_babel_ast(source_url: str, text: str, helper: str, cache: ReconCache) -> dict[str, Any]:
    if not helper or not Path(helper).is_file() or not shutil.which("node"):
        return {"apis": [], "imports": [], "routes": [], "baseUrls": [], "codeSlices": [], "parseErrors": ["babel_ast_unavailable"]}
    digest = hashlib.sha256(text.encode("utf-8", "ignore")).hexdigest()
    cached = cache.ast.get(digest)
    if cached is not None:
        # Source URLs are presentation metadata and can differ for identical CDN content.
        result = json.loads(json.dumps(cached))
        for section in ("apis", "routes"):
            for item in result.get(section, []):
                item["source"] = source_url
        return result
    try:
        completed = subprocess.run(
            [shutil.which("node") or "node", node_compatible_path(helper)],
            input=json.dumps({"url": source_url, "source": text}, ensure_ascii=False),
            text=True,
            capture_output=True,
            timeout=25,
            check=False,
        )
        if completed.returncode != 0:
            raise RuntimeError(completed.stderr.strip()[:500] or f"node exit {completed.returncode}")
        result = json.loads(completed.stdout)
        cache.ast[digest] = result
        return result
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        return {"apis": [], "imports": [], "routes": [], "baseUrls": [], "codeSlices": [], "parseErrors": [str(error)[:500]]}


def run_browser_runtime(
    source_url: str, helper: str, timeout: float, auth_session: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Render and deterministically explore one origin in an installed Chromium browser."""
    unavailable = {
        "available": False, "frameworks": [], "routes": [], "scripts": [],
        "requests": [], "links": [], "forms": [], "states": [], "actions": [],
        "features": [], "blockedRequests": [], "coverage": {},
        "authSessionValidation": {"applied": False, "valid": False, "clearSessionInvalid": False, "wafDetected": False},
        "stopReason": "unavailable", "errors": [],
    }
    if not helper or not Path(helper).is_file():
        return {**unavailable, "errors": ["runtime_helper_missing"]}
    node = shutil.which("node")
    if not node:
        return {**unavailable, "errors": ["node_unavailable"]}
    try:
        exploration_timeout_ms = max(30_000, min(90_000, int(timeout * 3000)))
        completed = subprocess.run(
            [node, node_compatible_path(helper)],
            input=json.dumps({
                "url": source_url,
                "timeoutMs": max(5_000, int(timeout * 1000)),
                "explorationTimeoutMs": exploration_timeout_ms,
                "maxActions": 24,
                "maxStates": 12,
                "maxDepth": 2,
                "maxRequests": 800,
                "settleMs": 750,
                "authSession": auth_session or {},
            }, ensure_ascii=False),
            text=True,
            capture_output=True,
            timeout=max(60, int(exploration_timeout_ms / 1000) + 30),
            check=False,
        )
        if completed.returncode != 0:
            reason = completed.stderr.strip()[:1000] or f"runtime helper exit {completed.returncode}"
            return {**unavailable, "errors": [reason]}
        result = json.loads(completed.stdout)
        return result if isinstance(result, dict) else {**unavailable, "errors": ["invalid_runtime_result"]}
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        return {**unavailable, "errors": [str(error)[:1000]]}


def run_jsluice(source_url: str, text: str) -> list[dict[str, Any]]:
    """Use jsluice's tree-sitter matchers when the optional CLI is installed."""
    executable = shutil.which("jsluice")
    if not executable:
        return []
    try:
        completed = subprocess.run(
            [executable, "urls", "--raw-input", "--include-source", "--placeholder", "<expr>"],
            input=text,
            text=True,
            capture_output=True,
            timeout=25,
            check=False,
        )
        if completed.returncode != 0:
            return []
        records = []
        for line in completed.stdout.splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            url = str(value.get("url", "")).strip()
            method = str(value.get("method", "UNKNOWN") or "UNKNOWN").upper()
            match_type = str(value.get("type", ""))
            if not url or is_static_resource(url):
                continue
            if method == "UNKNOWN" and match_type.lower() in {"string", "stringliteral"} and not API_HINT.search(urlparse(url).path):
                continue
            records.append({
                "path": url,
                "method": method,
                "parameters": unique(list(value.get("queryParams", [])) + list(value.get("bodyParams", []))),
                "source": source_url,
                "confidence": "high" if method != "UNKNOWN" else "medium",
                "extractionEngine": "jsluice-tree-sitter",
                "evidence": str(value.get("source", ""))[:320],
            })
        return records
    except (OSError, subprocess.SubprocessError):
        return []


def fetch(url: str, timeout: float, max_bytes: int) -> tuple[str, dict[str, str], int, str]:
    request = Request(url, headers={"User-Agent": USER_AGENT, "Accept": "text/html,application/javascript,*/*"})
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    try:
        with urlopen(request, timeout=timeout, context=context) as response:
            raw = response.read() if max_bytes <= 0 else response.read(max_bytes + 1)[:max_bytes]
            if response.headers.get("Content-Encoding", "").lower() == "gzip":
                try:
                    raw = gzip.decompress(raw)
                except OSError:
                    pass
            charset = response.headers.get_content_charset() or "utf-8"
            return raw.decode(charset, "replace"), dict(response.headers.items()), response.status, response.geturl()
    except HTTPError as error:
        raw = error.read() if max_bytes <= 0 else error.read(max_bytes)
        return raw.decode("utf-8", "replace"), dict(error.headers.items()), error.code, error.geturl()
    except (URLError, OSError, TimeoutError) as error:
        raise RuntimeError(str(error)) from error


def _looks_like_json(body: str, content_type: str) -> bool:
    """Require a structured response before calling a candidate an API."""
    normalized = body.lstrip("\ufeff \t\r\n")
    if "json" in content_type.lower():
        try:
            json.loads(normalized)
            return True
        except (TypeError, ValueError):
            return False
    if normalized.startswith(("{", "[")):
        try:
            json.loads(normalized)
            return True
        except (TypeError, ValueError):
            return False
    return bool(re.match(r"^\s*(?:<\?xml\b|<(?:[A-Za-z_][\w.-]*:)?[A-Za-z_][\w.-]*(?:\s|>))", normalized, re.I))


def _is_html_document(body: str, content_type: str) -> bool:
    sample = body[:12_000].lower()
    return (
        "text/html" in content_type.lower()
        or bool(re.search(r"<!doctype\s+html|<html(?:\s|>)|<head(?:\s|>)", sample, re.I))
    )


def _html_signature(body: str) -> str:
    """Create a small stable signature for SPA fallback comparison."""
    normalized = re.sub(r"\s+", " ", body[:24_000]).strip().lower()
    return hashlib.sha256(normalized.encode("utf-8", "ignore")).hexdigest() if normalized else ""


def verify_api_candidate(
    candidate: dict[str, Any], page_url: str, timeout: float, entry_html: str,
) -> dict[str, Any]:
    """Probe a resolved endpoint without submitting a request body.

    A GET is intentionally used for every candidate: it can establish that a POST/PUT
    endpoint exists through a 405 response without replaying a state-changing action.
    """
    url = str(candidate.get("url", "")).strip()
    verification: dict[str, Any] = {
        "status": "unresolved", "verified": False, "probeMethod": "GET",
        "reason": "candidate_not_resolved",
    }
    parsed = urlparse(url)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc or any(
        token in url for token in ("${", "{{", "<expr>", "<id>")
    ):
        return verification
    try:
        body, headers, status, response_url = fetch(url, min(timeout, 6.0), 96_000)
    except Exception as error:
        verification.update({"status": "unreachable", "reason": str(error)[:240]})
        return verification

    content_type = headers.get("Content-Type", "")
    final = urlparse(response_url or url)
    page = urlparse(page_url)
    is_redirected = bool(final.netloc and final.netloc != parsed.netloc) or final.path.rstrip("/") != parsed.path.rstrip("/")
    html_document = _is_html_document(body, content_type)
    json_or_xml = _looks_like_json(body, content_type)
    same_as_entry = html_document and _html_signature(body) == _html_signature(entry_html)
    verification.update({
        "httpStatus": status,
        "contentType": content_type[:120],
        "resolvedUrl": response_url or url,
        "sameOrigin": final.netloc == page.netloc,
    })

    if status == 204:
        verification.update({"status": "verified", "verified": True, "reason": "empty_success_response"})
    elif status in {401, 403, 405} and not html_document and not is_redirected:
        verification.update({"status": "verified", "verified": True, "reason": f"http_{status}_endpoint_exists"})
    elif 200 <= status < 300 and not html_document and json_or_xml:
        verification.update({"status": "verified", "verified": True, "reason": "structured_success_response"})
    elif html_document:
        reason = "spa_fallback" if same_as_entry or is_redirected or (page.netloc == final.netloc and final.path in {"", "/"}) else "html_response"
        verification.update({"status": "rejected", "reason": reason})
    elif status >= 400:
        verification.update({"status": "rejected", "reason": f"http_{status}"})
    elif (
        str(candidate.get("method", "UNKNOWN")).upper() in {"POST", "PUT", "PATCH", "DELETE"}
        and candidate.get("confidence") == "high"
        and final.netloc == page.netloc
    ):
        # We deliberately do not replay a state-changing request. A plain 2xx
        # response to the safe GET therefore cannot disprove a strongly parsed
        # POST/PUT/PATCH/DELETE endpoint; keep it as actionable evidence for
        # Strix instead of labelling it a rejected API.
        verification.update({"status": "unresolved", "reason": "safe_get_inconclusive"})
    else:
        verification.update({"status": "rejected", "reason": "unstructured_response"})
    return verification


def verify_api_candidates(
    candidates: list[dict[str, Any]], page_url: str, entry_html: str,
    timeout: float, max_probes: int,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Return verified APIs separately from unresolved/rejected extraction clues."""
    ranked = sorted(
        candidates,
        key=lambda item: (
            item.get("method") in {"POST", "PUT", "PATCH", "DELETE"},
            item.get("confidence") == "high",
            business_signal_score(str(item.get("url", ""))),
        ),
        reverse=True,
    )
    verified, pending = [], []
    for index, candidate in enumerate(ranked):
        item = dict(candidate)
        if item.get("extractionEngine") == "browser-runtime":
            # A request observed in the rendered browser is already endpoint
            # ground truth. Re-probing it without the page's cookies/headers can
            # only downgrade valid evidence (for example to a login HTML page).
            check = {
                "status": "observed_runtime",
                "verified": True,
                "probeMethod": str(item.get("method", "GET")),
                "reason": "runtime_request_observed",
                "httpStatus": item.get("statusCode"),
                "contentType": item.get("contentType", ""),
                "sameOrigin": urlparse(str(item.get("url", ""))).netloc == urlparse(page_url).netloc,
            }
        elif max_probes <= 0 or index < max_probes:
            check = verify_api_candidate(item, page_url, timeout, entry_html)
        else:
            check = {"status": "not_probed", "verified": False, "probeMethod": "GET", "reason": "probe_budget"}
        item["verification"] = check
        if check.get("verified"):
            verified.append(item)
        else:
            pending.append(item)
    return verified, pending


def script_filename(url: str) -> str:
    return unquote(urlparse(url).path.rsplit("/", 1)[-1]).lower()


def classify_script(url: str) -> str:
    name = script_filename(url)
    if any(token in name for token in ("runtime", "manifest")):
        return "runtime"
    if any(token in name for token in ("vendor", "vendors", "node_modules")):
        return "vendor"
    if any(token in name for token in (
        "jquery", "bootstrap", "layui", "crypto-js", "cryptojs", "vue.", "vue.min",
        "react.", "react-dom", "angular.", "lodash", "moment", "axios", "polyfill",
        "echarts", "swiper", "highlight", "codemirror", "monaco", "require.js",
    )):
        return "library"
    if any(token in name for token in ("plugin", "extension")):
        return "plugin"
    # Webpack/Vite commonly hash the business entry files. Preserve their
    # identity before the generic hash/chunk rule below.
    if re.search(r"(?:^|[._-])(main|app|index|entry|bundle)(?:[._-]|$)", name):
        return "application"
    if re.search(r"(?:chunk|[._-][0-9a-f]{7,})", name):
        return "chunk"
    return "application"


def should_deep_analyze(script_type: str) -> bool:
    """Deep parse business/application bundles; keep common dependencies inventory-only."""
    return script_type in {"application", "chunk", "plugin"}


def should_fetch_for_discovery(script_type: str) -> bool:
    """Inventory every referenced script; later stages decide what Strix may receive."""
    return script_type in {"application", "chunk", "plugin", "runtime", "vendor", "library"}


def script_priority(url: str) -> int:
    name = script_filename(url)
    if classify_script(url) in {"vendor", "library", "runtime"}:
        return -100
    if re.search(r"(?:^|[._-])(main|app|index|entry|bundle)(?:[._-]|$)", name):
        return 100
    if "chunk" in name:
        return 50
    return 20


def referenced_script_urls(base_url: str, text: str) -> list[str]:
    """Find statically named lazy chunks without walking arbitrary page links."""
    candidates = [match.group(1) for match in DYNAMIC_SCRIPT.finditer(text)]
    candidates.extend(match.group(1) for match in RELATIVE_SCRIPT.finditer(text))
    candidates.extend(match.group(1) for match in ANY_SCRIPT_STRING.finditer(text))
    resolved = []
    for value in candidates:
        url = urljoin(base_url, value.replace("\\/", "/"))
        if urlparse(url).scheme in {"http", "https"}:
            resolved.append(url)
    return sorted(unique(resolved), key=script_priority, reverse=True)


def ast_import_urls(base_url: str, imports: list[str]) -> list[str]:
    resolved: list[str] = []
    for value in imports:
        value = value.strip().replace("\\/", "/")
        if not value or value.startswith(("node:", "data:")):
            continue
        # Package imports belong to vendor code. Only browser-resolvable files
        # and relative modules are fetched from the authorized target.
        if not value.startswith(("http://", "https://", "/", "./", "../")):
            continue
        parsed = urlparse(value)
        path = parsed.path
        if not re.search(r"\.(?:m?js|jsx|ts|tsx)$", path, re.I):
            if path.endswith("/"):
                continue
            value += ".js"
        url = urljoin(base_url, value)
        if urlparse(url).scheme in {"http", "https"}:
            resolved.append(url)
    return sorted(unique(resolved), key=script_priority, reverse=True)


def business_signal_score(text: str) -> int:
    score = 0
    for pattern, weight in [
        (r"\b(?:fetch|axios|XMLHttpRequest|\.ajax)\b", 3),
        (r"\b(?:api|auth|login|oauth|admin|upload|export|payment|order|graphql|token|session)\b", 3),
        (r"(?:register|registration|sign.?up|create.?account|注册|新規登録|회원가입|registrarse|registrieren)", 6),
        (r"(?:baseURL|apiBaseUrl|API_BASE_URL|/api/|/graphql)", 3),
        (r"(?:createRouter|vue-router|react-router|<Route|router\.)", 2),
        (r"(?:password|secret|access[_-]?key|private[_-]?key|Authorization)", 2),
    ]:
        if re.search(pattern, text, re.I):
            score += weight
    return score


def probable_vendor_bundle(url: str, text: str) -> bool:
    """Detect anonymous hashed dependency chunks without an API/business signal."""
    if len(text) < 180_000 or classify_script(url) != "chunk":
        return False
    dependency_markers = (
        "jquery", "lodash", "moment", "core-js", "regeneratorruntime", "webpackbootstrap",
        "react-dom", "vue.runtime", "echarts", "crypto-js", "zone.js",
    )
    marker_hits = sum(text.lower().count(marker) for marker in dependency_markers)
    return marker_hits >= 2 and business_signal_score(text) < 5


def _framework_version(corpus: str, framework: str, angular_version: str) -> tuple[str, str]:
    if framework == "Angular" and angular_version:
        return angular_version, "DOM ng-version"
    patterns: dict[str, list[str]] = {
        "Vue": [
            rf"(?:Vue(?:\.js)?|vue(?:\.runtime)?)(?:\s+|[/@_-])v?{SEMVER}",
            rf"(?:__VUE_VERSION__|Vue\.version|app\.version)\s*[:=]\s*[\"']{SEMVER}[\"']",
        ],
        "React": [
            rf"(?:React(?:DOM)?|react(?:-dom)?)(?:\s+|[/@_-])v?{SEMVER}",
            rf"(?:React\.version|version)\s*[:=]\s*[\"']{SEMVER}[\"'][^\n]{{0,180}}(?:react|React)",
        ],
        "Angular": [
            rf"(?:Angular|@angular/core)(?:\s+|[/@_-])v?{SEMVER}",
            rf"(?:VERSION\.full|version\.full)\s*[:=]\s*[\"']{SEMVER}[\"']",
        ],
        "Svelte": [rf"(?:Svelte|svelte)(?:\s+|[/@_-])v?{SEMVER}"],
        "Preact": [rf"(?:Preact|preact)(?:\s+|[/@_-])v?{SEMVER}"],
        "SolidJS": [rf"(?:Solid(?:JS)?|solid-js)(?:\s+|[/@_-])v?{SEMVER}"],
    }
    for pattern in patterns.get(framework, []):
        match = re.search(pattern, corpus, re.I)
        if match:
            return match.group(1), match.group(0)[:180]
    return "", ""


def framework_signals(
    html: str, scripts: list[tuple[str, str]], angular_version: str,
    runtime_frameworks: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    corpus = html + "\n" + "\n".join(f"{url}\n{body}" for url, body in scripts)
    candidates: list[dict[str, Any]] = []
    checks = [
        ("Vue", [(r"__VUE__|createApp\s*\(|new\s+Vue\s*\(|vue-router|__VUE_DEVTOOLS", 3), (r"data-v-[0-9a-f]{5,}|_createVNode", 1)]),
        ("React", [(r"ReactDOM|createRoot\s*\(|__REACT_DEVTOOLS|react-dom", 3), (r"__NEXT_DATA__|/_next/static/", 2)]),
        ("Angular", [(r"platformBrowserDynamic|@angular/core|ɵɵdefineComponent|ng-version", 3), (r"zone\.js|polyfills\.[0-9a-f]+\.js", 1)]),
        ("Svelte", [(r"SvelteComponent|svelte/internal|__svelte|data-svelte-h", 3)]),
        ("Preact", [(r"\bpreact\b|__PREACT_DEVTOOLS__", 3)]),
        ("SolidJS", [(r"\bsolid-js\b|_$HY\.|createSignal\s*\(", 3)]),
    ]
    for name, patterns in checks:
        score, evidence = 0, []
        for pattern, weight in patterns:
            match = re.search(pattern, corpus, re.I)
            if match:
                score += weight
                evidence.append(match.group(0)[:100])
        if score:
            candidates.append({"name": name, "confidence": "high" if score >= 3 else "medium", "score": score, "evidence": evidence})
    ecosystem = []
    for name, pattern in [
        ("Next.js", r"__NEXT_DATA__|/_next/static/"), ("Nuxt", r"__NUXT__|/_nuxt/"),
        ("Webpack", r"webpackChunk|__webpack_require__"), ("Vite", r"__vite__|/assets/index-[0-9a-f]+\.js"),
        ("RequireJS", r"requirejs|define\.amd"),
    ]:
        if re.search(pattern, corpus, re.I):
            ecosystem.append(name)
    libraries = []
    for name, pattern in [
        ("jQuery", r"jquery(?:-|\.)?(\d+(?:\.\d+){1,2})?|jQuery\.fn"),
        ("Bootstrap", r"bootstrap(?:\.min)?\.(?:js|css)|Bootstrap v(\d+(?:\.\d+){1,2})"),
        ("Element UI", r"element-ui|element-plus"),
        ("Ant Design", r"antd(?:\.min)?\.js|ant-design"),
        ("Layui", r"layui(?:\.js|\.use)"),
    ]:
        match = re.search(pattern, corpus, re.I)
        if match:
            version = next((group for group in match.groups() if group), "")
            libraries.append({"name": name, "version": version, "evidence": match.group(0)[:100]})
    for runtime in runtime_frameworks or []:
        name = str(runtime.get("name", "")).strip()
        if not name:
            continue
        existing = next((item for item in candidates if item["name"].lower() == name.lower()), None)
        evidence = str(runtime.get("evidence", "browser runtime"))[:180]
        if existing:
            existing["score"] = max(existing["score"], 5)
            existing["confidence"] = "high"
            if evidence and evidence not in existing["evidence"]:
                existing["evidence"].append(evidence)
        else:
            candidates.append({"name": name, "confidence": "high", "score": 5, "evidence": [evidence]})
    primary = max(candidates, key=lambda item: item["score"], default={"name": "Unknown", "confidence": "low", "score": 0, "evidence": []})
    if primary["name"] == "Unknown" and "RequireJS" in ecosystem:
        primary = {"name": "RequireJS / AMD", "confidence": "medium", "score": 1, "evidence": ["RequireJS/AMD module loader"]}
    elif primary["name"] == "Unknown" and libraries:
        primary = {"name": libraries[0]["name"], "confidence": "medium", "score": 1, "evidence": [libraries[0]["evidence"]]}
    runtime_primary = next(
        (item for item in runtime_frameworks or [] if str(item.get("name", "")).lower() == primary["name"].lower()),
        None,
    )
    runtime_version = str((runtime_primary or {}).get("version", "")).strip()
    static_version, version_evidence = _framework_version(corpus, primary["name"], angular_version)
    version = runtime_version or static_version
    version_source = "browser-runtime" if runtime_version else ("static-signature" if static_version else "")
    evidence = list(primary["evidence"])
    if version_evidence and version_evidence not in evidence:
        evidence.append(version_evidence)
    return {
        "framework": primary["name"], "version": version, "versionSource": version_source,
        "confidence": primary["confidence"], "evidence": evidence,
        "alternatives": candidates, "libraries": libraries, "buildTools": ecosystem,
    }


def nearby_parameters(text: str, position: int) -> list[str]:
    window = text[position : position + 800]
    matches = re.search(r"\b(?:params|data|body|query)\s*:\s*\{([^{}]{0,600})\}", window, re.I | re.S)
    if not matches:
        return []
    ignored = {"url", "method", "headers", "timeout", "baseURL"}
    return unique([key for key in OBJECT_KEYS.findall(matches.group(1)) if key not in ignored])


def normalize_endpoint(base: str, value: str) -> str:
    value = value.replace("\\/", "/").strip()
    if any(token in value for token in ("${", "{{", "<%")):
        return value
    return urljoin(base, value)


def endpoint_base(page_url: str, value: str, base_urls: list[str], client_base: str = "") -> str:
    if client_base:
        return urljoin(page_url, client_base)
    if not value.startswith("/") or not base_urls:
        return page_url
    page_host = urlparse(page_url).netloc
    same_host = [item for item in base_urls if urlparse(item).netloc in {"", page_host}]
    return (same_host or base_urls)[0]


API_PREFIX_MARKERS = {
    "api", "apis", "rest", "restapi", "openapi", "gateway", "gw",
    "backend", "service", "services", "graphql", "rpc",
}
ACTION_MARKERS = {
    "list", "page", "detail", "info", "query", "search", "get", "find",
    "create", "add", "save", "update", "edit", "delete", "remove", "submit",
    "upload", "download", "export", "import", "login", "logout", "refresh",
    "verify", "check", "enable", "disable",
}


def _version_segment(value: str) -> bool:
    return bool(re.fullmatch(r"v\d+(?:\.\d+)*|version\d+", value, re.I))


def _identifier_segment(value: str) -> bool:
    return bool(
        re.fullmatch(r"\d+", value)
        or re.fullmatch(r"[0-9a-f]{8}-[0-9a-f-]{27,}", value, re.I)
        or re.fullmatch(r"[0-9a-f]{16,}", value, re.I)
        or re.fullmatch(r"[A-Za-z0-9_-]{24,}", value)
    )


def normalized_endpoint_path(pathname: str) -> str:
    segments = [segment for segment in pathname.split("/") if segment]
    if not segments:
        return "/"
    return "/" + "/".join("{id}" if _identifier_segment(segment) else segment for segment in segments)


def infer_api_split(pathname: str) -> dict[str, str]:
    segments = [segment for segment in pathname.split("/") if segment]
    if not segments:
        return {"apiPrefix": "/", "businessEndpoint": "/", "splitReason": "root"}
    split_index = -1
    reason = "full-path"
    for index, segment in enumerate(segments):
        lowered = segment.lower()
        if lowered not in API_PREFIX_MARKERS:
            continue
        split_index = index
        reason = f"marker:{lowered}"
        cursor = index + 1
        while cursor < len(segments):
            next_value = segments[cursor]
            if next_value.lower() in API_PREFIX_MARKERS or _version_segment(next_value):
                split_index = cursor
                cursor += 1
            else:
                break
        break
    if split_index < 0:
        for index, segment in enumerate(segments[:3]):
            if _version_segment(segment):
                split_index = index
                reason = f"version:{segment}"
                break
    if split_index < 0 and len(segments) >= 3 and re.search(r"(?:service|server|gateway)$", segments[0], re.I):
        split_index = 0
        reason = "service-prefix"
    if split_index < 0:
        return {"apiPrefix": "/", "businessEndpoint": "/" + "/".join(segments), "splitReason": reason}
    prefix = "/" + "/".join(segments[:split_index + 1])
    business = segments[split_index + 1:]
    return {"apiPrefix": prefix, "businessEndpoint": "/" + "/".join(business) if business else "/", "splitReason": reason}


def _common_path_prefix(paths: list[str]) -> str:
    rows = [[segment for segment in path.split("/") if segment] for path in paths]
    if len(rows) < 2:
        return "/"
    common = []
    for values in zip(*rows):
        if len(set(values)) != 1 or _identifier_segment(values[0]) or values[0].lower() in ACTION_MARKERS:
            break
        common.append(values[0])
    return "/" + "/".join(common) if common else "/"


def _evidence_records(ast_outputs: list[dict[str, Any]], kind: str) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for output in ast_outputs:
        values = output.get("stringEvidence", {}).get(kind, [])
        for value in values if isinstance(values, list) else []:
            if isinstance(value, dict) and str(value.get("value", "")).strip():
                records.append(dict(value))
    return unique(records, key=lambda item: f"{item.get('value')}|{item.get('source')}|{item.get('label')}")


def build_api_intelligence(
    page_url: str,
    runtime_requests: list[dict[str, Any]],
    ast_outputs: list[dict[str, Any]],
    api_records: list[dict[str, Any]],
) -> dict[str, Any]:
    """Split observed paths and reconstruct only evidence-backed API candidates.

    Runtime requests remain ground truth. Static fragments are combined only when an
    explicit client base/prefix or a prefix supported by multiple observed requests is
    available. This preserves AntiDebug_Breaker's useful reconstruction behavior without
    turning every JavaScript string into a speculative URL cartesian product.
    """
    page = urlparse(page_url)
    page_origin = f"{page.scheme}://{page.netloc}" if page.scheme and page.netloc else page_url
    base_evidence = _evidence_records(ast_outputs, "baseUrls")
    prefix_evidence = _evidence_records(ast_outputs, "apiPrefixes")
    business_evidence = _evidence_records(ast_outputs, "businessPaths")
    storage_evidence = _evidence_records(ast_outputs, "storageReferences")
    observed: list[dict[str, Any]] = []
    origin_paths: dict[str, list[str]] = {}
    for request in runtime_requests:
        if str(request.get("resourceType", "")).lower() not in {"xhr", "fetch", "eventsource"}:
            continue
        parsed = urlparse(str(request.get("url", "")))
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            continue
        origin = f"{parsed.scheme}://{parsed.netloc}"
        origin_paths.setdefault(origin, []).append(parsed.path)
        split = infer_api_split(parsed.path)
        observed.append({
            "method": str(request.get("method", "GET")).upper(),
            "url": str(request.get("url", "")),
            "origin": origin,
            **split,
            "normalizedPath": normalized_endpoint_path(parsed.path),
            "validated": True,
            "confidence": 1.0,
            "source": "browser-runtime",
            "stateId": request.get("stateId", ""),
            "actionId": request.get("actionId", ""),
            "feature": request.get("feature", ""),
        })

    clients: list[dict[str, Any]] = []
    client_keys: set[str] = set()
    for record in observed:
        prefix = str(record["apiPrefix"])
        key = f"{record['origin']}|{prefix}"
        if prefix != "/" and key not in client_keys:
            support = sum(1 for item in observed if item["origin"] == record["origin"] and item["apiPrefix"] == prefix)
            clients.append({"origin": record["origin"], "apiPrefix": prefix, "requestCount": support, "confidence": 0.96, "source": record["splitReason"]})
            client_keys.add(key)
    for origin, paths in origin_paths.items():
        prefix = _common_path_prefix(paths)
        key = f"{origin}|{prefix}"
        if len(paths) >= 2 and prefix != "/" and key not in client_keys:
            clients.append({"origin": origin, "apiPrefix": prefix, "requestCount": len(paths), "confidence": 0.78, "source": "multi-request-common-prefix"})
            client_keys.add(key)

    explicit_bases: list[dict[str, Any]] = []
    for record in [*base_evidence, *storage_evidence]:
        raw = str(record.get("value", "")).strip()
        parsed = urlparse(urljoin(page_url, raw))
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            continue
        origin = f"{parsed.scheme}://{parsed.netloc}"
        prefix = parsed.path.rstrip("/") or "/"
        explicit_bases.append({"origin": origin, "apiPrefix": prefix, "record": record})
        key = f"{origin}|{prefix}"
        if key not in client_keys:
            clients.append({"origin": origin, "apiPrefix": prefix, "requestCount": 0, "confidence": 0.88, "source": "javascript-base-url", "evidence": record})
            client_keys.add(key)
    for record in prefix_evidence:
        raw = str(record.get("value", "")).strip()
        parsed = urlparse(urljoin(page_url, raw))
        origin = f"{parsed.scheme}://{parsed.netloc}" if parsed.scheme and parsed.netloc else page_origin
        prefix = parsed.path.rstrip("/") or "/"
        key = f"{origin}|{prefix}"
        if key not in client_keys:
            clients.append({"origin": origin, "apiPrefix": prefix, "requestCount": 0, "confidence": 0.82, "source": "javascript-api-prefix", "evidence": record})
            client_keys.add(key)

    direct_methods: dict[str, str] = {}
    for record in api_records:
        parsed = urlparse(str(record.get("url") or record.get("path") or ""))
        path = parsed.path or str(record.get("path", ""))
        if path:
            direct_methods.setdefault(normalized_endpoint_path(path), str(record.get("method", "UNKNOWN")).upper())

    reconstructions = list(observed)
    candidates: list[dict[str, Any]] = []
    trusted_clients = sorted(
        [client for client in clients if float(client.get("confidence", 0)) >= 0.78],
        key=lambda item: (float(item.get("confidence", 0)), int(item.get("requestCount", 0))),
        reverse=True,
    )[:8]
    for path_record in business_evidence[:48]:
        raw_path = str(path_record.get("value", "")).strip()
        if not raw_path.startswith("/") or not valid_api_path(raw_path, "UNKNOWN", "evidence-reconstruction"):
            continue
        raw_parsed = urlparse(raw_path)
        path_only = raw_parsed.path
        path_split = infer_api_split(path_only)
        for client in trusted_clients:
            origin = str(client.get("origin", page_origin))
            prefix = str(client.get("apiPrefix", "/")) or "/"
            if path_split["apiPrefix"] != "/" or prefix == "/" or path_only == prefix or path_only.startswith(prefix.rstrip("/") + "/"):
                combined_path = path_only
            else:
                combined_path = f"{prefix.rstrip('/')}/{path_only.lstrip('/')}"
            combined_path = re.sub(r"/{2,}", "/", combined_path)
            full_url = f"{origin}{combined_path}"
            if raw_parsed.query:
                full_url += "?" + raw_parsed.query
            confidence = min(0.9, float(client.get("confidence", 0)) - (0.02 if client.get("requestCount", 0) else 0.06))
            if confidence < 0.72:
                continue
            split = infer_api_split(combined_path)
            method = direct_methods.get(normalized_endpoint_path(path_only), "UNKNOWN")
            lineage = [
                {"type": "business-path", "value": raw_path, "source": path_record.get("source", ""), "line": path_record.get("line", 0)},
                {"type": str(client.get("source", "client-prefix")), "value": prefix, "requestCount": client.get("requestCount", 0)},
            ]
            candidate = {
                "path": combined_path,
                "url": full_url,
                "method": method,
                "parameters": unique([key for key, _ in parse_qsl(raw_parsed.query, keep_blank_values=True)]),
                "source": str(path_record.get("source", "javascript-string-evidence")),
                "confidence": "high" if confidence >= 0.84 else "medium",
                "reconstructionConfidence": round(confidence, 2),
                "extractionEngine": "evidence-reconstruction",
                "dynamic": False,
                "candidateOnly": True,
                "origin": origin,
                **split,
                "normalizedPath": normalized_endpoint_path(combined_path),
                "evidence": f"JS business path + {client.get('source', 'trusted client prefix')}",
                "evidenceLineage": lineage,
            }
            candidates.append(candidate)
            reconstructions.append({
                "method": method,
                "url": full_url,
                "origin": origin,
                **split,
                "expression": f"{origin} + {prefix} + {raw_path}",
                "validated": False,
                "confidence": round(confidence, 2),
                "source": "evidence-reconstruction",
                "evidenceLineage": lineage,
            })
            if len(candidates) >= 60:
                break
        if len(candidates) >= 60:
            break

    return {
        "clients": unique(clients, key=lambda item: f"{item.get('origin')}|{item.get('apiPrefix')}")[:20],
        "reconstructions": unique(reconstructions, key=lambda item: f"{item.get('method')}|{item.get('url')}")[:100],
        "candidates": unique(candidates, key=lambda item: f"{item.get('method')}|{item.get('url')}")[:60],
        "stringEvidence": {
            "baseUrls": base_evidence[:30],
            "apiPrefixes": prefix_evidence[:30],
            "businessPaths": business_evidence[:60],
            "storageReferences": storage_evidence[:30],
        },
        "policy": {
            "normalizerVersion": "api-intelligence-v1",
            "minimumReconstructionConfidence": 0.72,
            "maximumGeneratedCandidates": 60,
            "rule": "Only explicit JS client evidence or prefixes supported by multiple runtime requests may be recombined.",
        },
    }


def build_header_intelligence(
    runtime_requests: list[dict[str, Any]],
    ast_outputs: list[dict[str, Any]],
) -> dict[str, Any]:
    """Merge effective runtime headers and JavaScript-declared header clues.

    CDP ExtraInfo is the closest local source for browser-added headers. Protocol
    pseudo headers and transport-managed fields are kept as possible-only hints
    until they are actually observed; they are never presented as captured facts.
    """
    records: dict[str, dict[str, Any]] = {}

    def add(name: str, value: str, source: str, observed: bool, url: str = "", method: str = "", dynamic: bool = False, line: int = 0) -> None:
        clean_name = str(name or "").strip()
        if not clean_name or len(clean_name) > 160:
            return
        key = clean_name.lower()
        row = records.setdefault(key, {
            "name": clean_name,
            "observed": False,
            "declared": False,
            "sensitive": bool(re.search(r"^(?:authorization|proxy-authorization|cookie|set-cookie|x-api-key|api-key|x-auth-token)$", clean_name, re.I)),
            "sources": [],
            "values": [],
            "occurrences": 0,
        })
        row["observed"] = bool(row["observed"] or observed)
        row["declared"] = bool(row["declared"] or not observed)
        row["occurrences"] += 1
        if source not in row["sources"]:
            row["sources"].append(source)
        value_record = {
            "value": str(value or "")[:1200],
            "source": source,
            "url": str(url or "")[:1000],
            "method": str(method or "").upper(),
            "dynamic": bool(dynamic),
            "line": int(line or 0),
        }
        marker = f"{value_record['value']}|{source}|{value_record['url']}|{value_record['line']}"
        if marker not in {f"{item.get('value')}|{item.get('source')}|{item.get('url')}|{item.get('line')}" for item in row["values"]}:
            row["values"].append(value_record)
            row["values"] = row["values"][:12]

    runtime_api_requests = []
    for request in runtime_requests:
        if str(request.get("resourceType", "")).lower() not in {"xhr", "fetch", "websocket", "eventsource"}:
            continue
        runtime_api_requests.append(request)
        visible = request.get("headers", {}) if isinstance(request.get("headers"), dict) else {}
        effective = request.get("effectiveRequestHeaders", {}) if isinstance(request.get("effectiveRequestHeaders"), dict) else {}
        merged = dict(visible)
        merged.update(effective)
        extra_names = {str(value).lower() for value in request.get("extraInfoRequestHeaderNames", [])}
        for name, value in merged.items():
            add(
                str(name), str(value),
                "browser-extra-info" if str(name).lower() in extra_names else "runtime-request",
                True, str(request.get("url", "")), str(request.get("method", "GET")),
            )

    for output in ast_outputs:
        for record in output.get("headerEvidence", []) if isinstance(output.get("headerEvidence"), list) else []:
            if not isinstance(record, dict):
                continue
            add(
                str(record.get("name", "")), str(record.get("value", "")),
                str(record.get("sourceKind", "javascript-declared")), False,
                str(record.get("source", "")), "", bool(record.get("dynamic")), int(record.get("line", 0) or 0),
            )

    observed = sorted((row for row in records.values() if row["observed"]), key=lambda row: (-int(row["occurrences"]), row["name"].lower()))
    declared = sorted((row for row in records.values() if row["declared"] and not row["observed"]), key=lambda row: row["name"].lower())
    possible_names = [
        ("Host / :authority", "由 HTTP 版本与浏览器网络栈生成，CDP 未必作为普通 Header 暴露"),
        ("Content-Length", "存在请求体时通常由网络栈计算，前端 JavaScript 无法直接设置"),
        ("Cookie", "受 SameSite、域、路径和浏览器 Cookie 策略控制；只有实际发送时才算已观察"),
        ("Origin", "跨源请求或部分写请求由浏览器按上下文添加"),
        ("Referer", "由 Referrer-Policy 和导航上下文决定"),
        ("Sec-Fetch-Site / Mode / Dest", "浏览器 Fetch Metadata，请以 ExtraInfo 实际记录为准"),
        ("Sec-CH-UA-*", "Client Hints 取决于浏览器与服务端 Accept-CH 策略"),
    ]
    observed_names = {row["name"].lower() for row in observed}
    possible = [
        {"name": name, "observed": False, "possibleOnly": True, "reason": reason}
        for name, reason in possible_names
        if not any(part.strip().lower() in observed_names for part in name.split("/"))
    ]
    return {
        "observed": observed[:120],
        "declared": declared[:120],
        "possibleBrowserManaged": possible,
        "summary": {
            "runtimeRequestCount": len(runtime_api_requests),
            "observedHeaderCount": len(observed),
            "declaredOnlyHeaderCount": len(declared),
            "extraInfoHeaderCount": sum(1 for row in observed if "browser-extra-info" in row.get("sources", [])),
        },
        "policy": {
            "observedIsFact": True,
            "declaredNeedsRuntimeConfirmation": True,
            "possibleBrowserManagedIsNotEvidence": True,
        },
    }


def is_static_resource(value: str) -> bool:
    path = urlparse(value).path.lower()
    return bool(re.search(r"\.(?:avif|bmp|css|eot|gif|ico|jpe?g|js|map|mp3|mp4|pdf|png|svg|ttf|webp|woff2?)$", path))


def normalize_route_path(value: str, parent: str = "") -> str:
    value = value.strip().replace("\\/", "/")
    if not value:
        return parent or "/"
    if value in {"*", "/*"}:
        return (parent.rstrip("/") if parent else "") + "/*"
    if value.startswith("/"):
        return re.sub(r"/{2,}", "/", value)
    base = parent.rstrip("/") if parent else ""
    return re.sub(r"/{2,}", "/", f"{base}/{value}")


def valid_frontend_route(value: str) -> bool:
    value = value.strip()
    if not value or len(value) > 1000 or not value.startswith("/") or value.startswith("//"):
        return False
    if re.search(r"\s|[<>]|(?:^|/)(?:M|path|d)(?:/|$)", value):
        return False
    if is_static_resource(value):
        return False
    # SVG path data often leaks through minified objects named `path`.
    return not bool(re.fullmatch(r"/[MmLlHhVvCcSsQqTtAaZz0-9.,+\-]+", value))


def registration_signal(value: str, context: str = "") -> dict[str, Any] | None:
    decoded = unquote(str(value or "")).replace("\\/", "/").strip()
    searchable = f"{decoded} {context}".strip()
    core = REGISTRATION_CORE.search(decoded)
    # Labels come from rendered link/form text. Do not treat an unrelated URL
    # such as `/reports/registration-report` as a registration entry merely
    # because its slug contains the English noun.
    label = REGISTRATION_LABELS.search(context)
    support = REGISTRATION_SUPPORT.search(decoded)
    if core or label:
        matches = unique([
            match.group(0).strip()
            for match in (core, label)
            if match and match.group(0).strip()
        ])
        return {
            "detected": True,
            "category": "account_registration",
            "confidence": "high" if core and label else "medium",
            "matchedTerms": matches,
            "label": "注册 / 创建账户",
        }
    if support:
        return {
            "detected": True,
            "category": "registration_support",
            "confidence": "medium",
            "matchedTerms": [support.group(0).strip()],
            "label": "注册辅助接口",
        }
    return None


def mark_registration(records: list[dict[str, Any]], value_fields: tuple[str, ...]) -> list[dict[str, Any]]:
    for record in records:
        value = next((str(record.get(field, "")) for field in value_fields if record.get(field)), "")
        context = " ".join(str(record.get(field, "")) for field in ("evidence", "note", "context", "title"))
        signal = registration_signal(value, context)
        if signal:
            record["registration"] = signal
            record["highValue"] = True
    return records


def registration_entrypoints(
    page_url: str,
    apis: list[dict[str, Any]],
    api_candidates: list[dict[str, Any]],
    routes: list[dict[str, Any]],
    links: list[Any],
    forms: list[dict[str, str]],
    runtime_requests: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []

    def add(value: str, source_type: str, source: str, method: str = "GET", context: str = "", verification: Any = None) -> None:
        signal = registration_signal(value, context)
        if not signal:
            return
        resolved = value if urlparse(value).scheme in {"http", "https"} else urljoin(page_url, value)
        entries.append({
            "url": resolved,
            "path": urlparse(resolved).path or value,
            "method": method or "GET",
            "sourceType": source_type,
            "source": source,
            "category": signal["category"],
            "confidence": signal["confidence"],
            "matchedTerms": signal["matchedTerms"],
            "title": signal["label"],
            "highValue": True,
            "verification": verification or {},
            "note": "高价值账户入口；仅进行授权人工验证，前置侦察不会自动提交注册。",
        })

    for record in [*apis, *api_candidates]:
        add(
            str(record.get("url") or record.get("path") or ""),
            "api",
            str(record.get("source", "")),
            str(record.get("method", "UNKNOWN")),
            str(record.get("evidence", "")),
            record.get("verification"),
        )
    for record in routes:
        add(str(record.get("path", "")), "route", str(record.get("source", "")), "GET", str(record.get("evidence", "")))
    for link in links:
        if isinstance(link, dict):
            add(str(link.get("url", "")), "link", page_url, "GET", str(link.get("text", "")))
        else:
            add(str(link), "link", page_url)
    for form in forms:
        add(
            str(form.get("action", "")),
            "form",
            page_url,
            str(form.get("method", "POST")),
            " ".join(str(form.get(key, "")) for key in ("id", "name", "class", "text")),
        )
    for request in runtime_requests:
        add(
            str(request.get("url", "")),
            "runtime-request",
            "browser-runtime",
            str(request.get("method", "GET")),
            str(request.get("resourceType", "")),
        )
    return unique(entries, key=lambda item: f"{item['method']}|{item['url']}|{item['category']}")


def extract_regex_routes(source: str, text: str) -> list[dict[str, str]]:
    """Accept fallback routes only when the surrounding object is route-shaped."""
    records: list[dict[str, str]] = []
    for match in ROUTE_PATH.finditer(text):
        value = match.group(1).strip()
        if not valid_frontend_route(value):
            continue
        jsx_route = "<Route" in match.group(0)
        prefix = text[max(0, match.start() - 180):match.start()]
        suffix = text[match.end():min(len(text), match.end() + 360)]
        object_start = prefix.rfind("{")
        object_end = suffix.find("}")
        window = (prefix[object_start:] if object_start >= 0 else prefix[-80:]) + match.group(0)
        window += suffix[:object_end + 1] if object_end >= 0 else suffix[:180]
        route_shape = re.search(
            r"\b(?:component|children|redirect|element|loader|action|loadChildren|beforeEnter|meta)\b\s*[:=]",
            window,
            re.I,
        )
        route_container = re.search(r"\b(?:routes|children)\b\s*[:=]\s*\[[^\]]*$", prefix, re.I)
        if not jsx_route and not route_shape and not route_container:
            continue
        records.append({
            "path": value,
            "type": "frontend",
            "source": source,
            "confidence": "high" if jsx_route else "medium",
            "extractionEngine": "route-structure-fallback",
        })
    return records


def valid_api_path(value: str, method: str = "UNKNOWN", engine: str = "") -> bool:
    value = value.strip()
    if not value or len(value) > 700 or is_static_resource(value):
        return False
    normalized_dynamic = re.sub(r"<[A-Za-z_$][\w$]*>", "", value).replace("<expr>", "")
    if re.search(r"\s|[<>]", normalized_dynamic):
        return False
    parsed = urlparse(value)
    path = parsed.path or value
    if path in {"", "/"} or path.lower() in {"path", "d", "m"}:
        return False
    if value.startswith("/") and not value.startswith("//"):
        if method == "UNKNOWN" and engine in {"", "string-heuristic", "legacy"}:
            return bool(API_HINT.search(path) or re.search(r"(?:oauth|sso|upload|download|export|\.do$|\.action$|\.php$|\.ashx$|\.aspx?$)", path, re.I))
        return True
    return parsed.scheme in {"http", "https"} or any(token in value for token in ("${", "<expr>", "<id>"))


def extract_apis(base: str, source_url: str, text: str, base_urls: list[str]) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    direct_values: set[str] = set()
    for regex, method_group, url_group, default_method in [
        (FETCH_CALL, None, 1, "GET"), (AXIOS_CALL, 1, 2, "GET"), (AJAX_URL, None, 1, "UNKNOWN")
    ]:
        for match in regex.finditer(text):
            value = match.group(url_group)
            if is_static_resource(value) or value.startswith(("mailto:", "tel:", "javascript:")):
                continue
            if regex is AJAX_URL:
                window = text[max(0, match.start() - 220): match.end() + 500]
                if not re.search(r"\b(?:ajax|axios|request|fetch|XMLHttpRequest|method|headers|baseURL)\b", window, re.I):
                    continue
                if re.search(r"\b(?:component|redirect|children|route|template)\s*:", window, re.I) and not re.search(r"\b(?:method|headers|params|data|body)\s*:", window, re.I):
                    continue
            direct_values.add(value)
            method = match.group(method_group).upper() if method_group else default_method
            if regex is FETCH_CALL:
                method_match = re.search(r"\bmethod\s*:\s*[\"'](GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)[\"']", text[match.end():match.end()+500], re.I)
                if method_match:
                    method = method_match.group(1).upper()
            if not valid_api_path(value, method, "regex-fallback"):
                continue
            params = [key for key, _ in parse_qsl(urlparse(value).query, keep_blank_values=True)]
            params.extend(nearby_parameters(text, match.end()))
            resolved_base = endpoint_base(base, value, base_urls)
            records.append({
                "path": value, "url": normalize_endpoint(resolved_base, value), "method": method,
                "parameters": unique(params), "source": source_url, "confidence": "high",
                "evidence": match.group(0)[:240], "extractionEngine": "regex-fallback",
            })
    for match in QUOTED_PATH.finditer(text):
        value = match.group(1)
        if value in direct_values:
            continue
        parsed = urlparse(value)
        if "#" in value or is_static_resource(value):
            continue
        api_host = bool(parsed.netloc and re.search(r"(?:^|[.-])(?:api|interface|gateway|sso)(?:[.-]|$)", parsed.netloc, re.I))
        server_endpoint = bool(re.search(r"\.(?:json|do|action|ashx|aspx?|php)$", parsed.path, re.I) and parsed.query)
        if not API_HINT.search(parsed.path) and not api_host and not server_endpoint:
            continue
        records.append({
            "path": value, "url": normalize_endpoint(base, value), "method": "UNKNOWN",
            "parameters": unique([key for key, _ in parse_qsl(parsed.query, keep_blank_values=True)]),
            "source": source_url, "confidence": "medium", "evidence": match.group(0)[:240],
            "extractionEngine": "string-heuristic",
        })
    return unique(records, key=lambda item: f"{item['method']}|{item['url']}|{','.join(item['parameters'])}")


def ai_fallback_evidence(
    framework: dict[str, Any], scripts: list[tuple[str, str]], apis: list[dict[str, Any]],
    ast_outputs: list[dict[str, Any]],
) -> dict[str, Any]:
    """Prepare a small initial packet plus bounded AST slices for on-demand model reads."""
    framework_name = str(framework.get("framework", "Unknown"))
    if framework_name in {"", "Unknown"}:
        return {}
    ast_apis = [item for item in apis if str(item.get("extractionEngine", "")).startswith("babel-ast")]
    concrete = [item for item in apis if not item.get("dynamic") and item.get("path") not in {"", "/"}]
    if len(concrete) >= 5 and len(ast_apis) >= 2:
        return {}
    markers = re.compile(
        r"(?:axios|fetch|XMLHttpRequest|baseURL|apiBaseUrl|graphql|router\.|vue-router|react-router|"
        r"login|logout|auth|token|session|admin|upload|download|export|payment|order|permission)", re.I,
    )
    ranked = sorted(
        scripts,
        key=lambda item: (business_signal_score(item[1]), script_priority(item[0])),
        reverse=True,
    )
    snippets: list[dict[str, str]] = []
    code_slices: list[dict[str, Any]] = []
    seen: set[str] = set()
    total_chars = 0

    ast_slices = [
        dict(item)
        for output in ast_outputs
        for item in output.get("codeSlices", [])
        if isinstance(item, dict) and str(item.get("context", "")).strip()
    ]
    ast_slices.sort(
        key=lambda item: (
            str(item.get("kind", "")) == "network-call",
            business_signal_score(f"{item.get('marker', '')} {item.get('context', '')[:4000]}"),
            -len(str(item.get("context", ""))),
        ),
        reverse=True,
    )
    for item in ast_slices:
        context = str(item.get("context", "")).replace("\x00", "")
        slice_id = str(item.get("id", ""))[:32]
        if not slice_id or slice_id in seen:
            continue
        remaining = 72_000 - total_chars
        if remaining < 1_000 or len(code_slices) >= 8:
            break
        context = context[:min(14_000, remaining)]
        focus = max(0, min(len(context), int(item.get("focusStart", 0) or 0)))
        initial = context[max(0, focus - 480):min(len(context), focus + 720)]
        code_slices.append({
            "id": slice_id,
            "source": str(item.get("source", ""))[:1000],
            "kind": str(item.get("kind", "business-flow"))[:80],
            "marker": str(item.get("marker", ""))[:120],
            "start": int(item.get("start", 0) or 0),
            "end": int(item.get("end", 0) or 0),
            "context": context,
        })
        snippets.append({
            "sliceId": slice_id,
            "source": str(item.get("source", ""))[:1000],
            "marker": str(item.get("marker", ""))[:80],
            "context": initial,
        })
        seen.add(slice_id)
        total_chars += len(context)

    # Babel may be unavailable or unable to parse a damaged bundle. Keep the
    # old marker windows as a deterministic fallback, but never expose the file.
    fallback_chars = 0
    for source, body in ranked[:4]:
        if code_slices:
            break
        matches = list(markers.finditer(body))
        for match in matches[:12]:
            start = max(0, match.start() - 520)
            end = min(len(body), match.end() + 680)
            snippet = body[start:end].replace("\x00", "")
            marker = hashlib.sha256(snippet.encode("utf-8", "ignore")).hexdigest()[:16]
            if marker in seen:
                continue
            remaining = 12_000 - fallback_chars
            if remaining < 320:
                break
            snippet = snippet[:remaining]
            snippets.append({
                "sliceId": marker,
                "source": source,
                "marker": match.group(0)[:80],
                "context": snippet,
            })
            code_slices.append({
                "id": marker,
                "source": source,
                "kind": "marker-window",
                "marker": match.group(0)[:120],
                "start": start,
                "end": start + len(snippet),
                "context": snippet,
            })
            seen.add(marker)
            fallback_chars += len(snippet)
            if len(code_slices) >= 8:
                break
        if fallback_chars >= 12_000:
            break
    if not snippets:
        return {}
    return {
        "enabled": True,
        "framework": framework_name,
        "reason": "框架已识别，但已验证接口不足；先读取小型证据，确有依赖缺口时再按索引读取有限函数/模块切片。",
        "maxChars": total_chars or fallback_chars,
        "maxSliceReads": 3,
        "maxCumulativeSliceChars": min(72_000, total_chars or fallback_chars),
        "snippets": snippets,
        "codeSlices": code_slices,
        "localOnlySourceFiles": [source for source, _ in ranked[:4]],
    }


def normalize_ast_apis(page_url: str, records: list[dict[str, Any]], base_urls: list[str]) -> list[dict[str, Any]]:
    result = []
    for record in records:
        item = dict(record)
        method = str(item.get("method", "UNKNOWN")).upper()
        if method == "UNKNOWN":
            # Compatibility for cached/older AST output and uncommon request
            # wrappers. The evidence is a bounded source slice, not arbitrary
            # response text, so this remains a deterministic extraction.
            method_match = re.search(
                r"\b(?:method|type)\s*:\s*[\"'](GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)[\"']",
                str(item.get("evidence", "")),
                re.I,
            )
            if method_match:
                item["method"] = method_match.group(1).upper()
        value = str(record.get("path", "")).strip()
        if not valid_api_path(value, str(item.get("method", "UNKNOWN")), str(item.get("extractionEngine", ""))):
            continue
        client_base = str(record.get("clientBaseUrl", "")).strip()
        resolved_base = endpoint_base(page_url, value, base_urls, client_base)
        if client_base and not urlparse(value).scheme:
            parsed_base = urlparse(resolved_base)
            base_path = parsed_base.path.rstrip("/")
            if value.startswith("/") and base_path and (value == base_path or value.startswith(base_path + "/")):
                item["url"] = f"{parsed_base.scheme}://{parsed_base.netloc}{value}"
            else:
                item["url"] = f"{resolved_base.rstrip('/')}/{value.lstrip('/')}"
        else:
            item["url"] = normalize_endpoint(resolved_base, value)
        item.pop("clientBaseUrl", None)
        result.append(item)
    return result


def sensitive_patterns() -> list[tuple[str, str, re.Pattern[str]]]:
    flags = re.I
    return [
        ("private_key", "high", re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")),
        ("aws_access_key", "high", re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b")),
        ("alibaba_access_key", "high", re.compile(r"\bLTAI[A-Za-z0-9]{12,24}\b")),
        ("tencent_access_key", "high", re.compile(r"\bAKID[A-Za-z0-9]{13,40}\b")),
        ("google_api_key", "high", re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")),
        ("google_oauth_token", "high", re.compile(r"\bya29\.[0-9A-Za-z_-]{20,}\b")),
        ("github_token", "high", re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{36,255}|github_pat_[A-Za-z0-9_]{40,255})\b")),
        ("gitlab_token", "high", re.compile(r"\bglpat-[A-Za-z0-9_-]{20,255}\b")),
        ("slack_token", "high", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{20,255}\b")),
        ("stripe_secret_key", "high", re.compile(r"\bsk_live_[A-Za-z0-9]{16,255}\b")),
        ("sendgrid_api_key", "high", re.compile(r"\bSG\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\b")),
        ("npm_token", "high", re.compile(r"\bnpm_[A-Za-z0-9]{30,255}\b")),
        ("jwt", "high", re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{8,}\b")),
        ("bearer_token", "high", re.compile(r"\bBearer\s+([A-Za-z0-9._~+/=-]{20,500})", flags)),
        ("database_password", "high", re.compile(r"(?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|redis)://[^\s:/@]+:([^\s/@]{8,160})@", flags)),
        ("webhook", "high", re.compile(r"https?://[^\s\"']+(?:webhook|hooks)[^\s\"']*", flags)),
        ("cloud_access_key", "high", re.compile(r"(?:jdcloud|baidu|bce|bytedance|volcengine|ksyun|kingsoft|google)[A-Za-z0-9_.-]{0,30}(?:access_?key|secret_?key|ak|sk)\s*[:=]\s*[\"']([^\"']{8,160})[\"']", flags)),
        ("password_assignment", "high", re.compile(r"(?:password|passwd|pwd|secret|sessionkey)\s*[:=]\s*[\"']([^\"']{6,160})[\"']", flags)),
        ("wechat_appid", "medium", re.compile(r"\bwx[0-9a-f]{16}\b", flags)),
        ("corp_id", "medium", re.compile(r"(?:corp_?id|corpid)\s*[:=]\s*[\"']([^\"']{4,100})[\"']", flags)),
        ("email", "medium", re.compile(r"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b", flags)),
        ("cn_phone", "medium", re.compile(r"(?<!\d)1[3-9]\d{9}(?!\d)")),
        ("cn_id", "medium", re.compile(r"(?<!\d)\d{17}[0-9Xx](?!\d)")),
        ("mac_address", "medium", re.compile(r"\b(?:[0-9A-F]{2}:){5}[0-9A-F]{2}\b", flags)),
        ("ip_address", "medium", re.compile(r"(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])")),
        ("encryption_key", "medium", re.compile(r"(?:encrypt(?:ion)?_?key|aes_?key|des_?key|crypto_?key)\s*[:=]\s*[\"']([^\"']{8,160})[\"']", flags)),
        ("credential_in_url", "high", re.compile(r"https?://[^\s\"'`<>]{1,500}[?&](?:access_?token|api_?key|secret|password|passwd|signature)=([^&#\s\"'`<>]{6,200})", flags)),
    ]


def context_window(text: str, start: int, end: int) -> str:
    """Return at most 200 surrounding characters for local manual review."""
    center = (start + end) // 2
    left = max(0, center - 100)
    right = min(len(text), left + 200)
    left = max(0, right - 200)
    return text[left:right].replace("\r", " ").replace("\n", " ")[:200]


def extract_runtime_signals(source: str, text: str) -> list[dict[str, str]]:
    signals: list[dict[str, str]] = []
    for kind, label, pattern in RUNTIME_SIGNAL_PATTERNS:
        match = pattern.search(text)
        if not match:
            continue
        signals.append({
            "type": kind,
            "label": label,
            "source": source,
            "evidence": match.group(0)[:180],
            "context": context_window(text, match.start(), match.end()),
            "nextStep": "Use a narrowly scoped browser/runtime hook only if a high-value candidate needs live request or crypto evidence.",
        })
    return signals


def extract_crypto_signals(source: str, text: str) -> list[dict[str, str]]:
    """Classify locally visible crypto usage. These records are display-only."""
    signals: list[dict[str, str]] = []
    for category, algorithm, operation, confidence, pattern in CRYPTO_SIGNAL_PATTERNS:
        match = pattern.search(text)
        if not match:
            continue
        signals.append({
            "category": category,
            "algorithm": algorithm,
            "operation": operation,
            "confidence": confidence,
            "source": source,
            "evidence": match.group(0)[:180],
            "context": context_window(text, match.start(), match.end()),
            "localOnly": True,
        })
    return signals


def select_runtime_hook(
    signals: list[dict[str, str]], apis: list[dict[str, Any]], routes: list[dict[str, Any]]
) -> dict[str, str]:
    """Recommend one narrow hook only when static evidence leaves a useful gap."""
    by_type = {signal["type"]: signal for signal in signals}
    if not apis and "network_runtime" in by_type:
        return {
            "hook": "fetch_xhr",
            "reason": "Requests are computed at runtime and static parsing found no concrete API candidate.",
            "source": by_type["network_runtime"]["source"],
        }
    if not routes and "route_runtime" in by_type:
        return {
            "hook": "router",
            "reason": "A client router is present but static parsing found no concrete route.",
            "source": by_type["route_runtime"]["source"],
        }
    auth_api = any(re.search(r"auth|login|oauth|token|session", item.get("url", ""), re.I) for item in apis)
    if auth_api and "browser_storage" in by_type:
        return {
            "hook": "storage",
            "reason": "Authentication endpoints and browser storage are both present; inspect only the relevant credential key.",
            "source": by_type["browser_storage"]["source"],
        }
    return {}


def valid_cn_id(value: str) -> bool:
    if not re.fullmatch(r"\d{17}[0-9Xx]", value):
        return False
    try:
        year, month, day = int(value[6:10]), int(value[10:12]), int(value[12:14])
        datetime(year, month, day)
    except ValueError:
        return False
    weights = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2]
    checks = "10X98765432"
    return checks[sum(int(value[index]) * weights[index] for index in range(17)) % 11] == value[-1].upper()


def plausible_secret(value: str) -> bool:
    stripped = value.strip()
    if len(stripped) < 8 or stripped.lower() in {"password", "passwd", "undefined", "null", "example", "changeme", "12345678"}:
        return False
    if stripped.startswith("+") or stripped.endswith("+") or any(token in stripped for token in ("${", "{{", "}}", "<%", "%>")):
        return False
    return bool(re.search(r"[A-Za-z]", stripped) and re.search(r"\d|[^A-Za-z]", stripped))


def sensitive_context(text: str, start: int, end: int, radius: int = 180) -> str:
    return text[max(0, start - radius): min(len(text), end + radius)]


def plausible_ip(text: str, match: re.Match[str], value: str) -> tuple[bool, str]:
    try:
        address = ipaddress.ip_address(value)
    except ValueError:
        return False, ""
    before = text[match.start() - 1:match.start()] if match.start() else ""
    after = text[match.end():match.end() + 1]
    if before in ".0123456789" or after in ".0123456789":
        return False, ""
    context = sensitive_context(text, match.start(), match.end()).lower()
    svg_markers = (
        "<path", "viewbox", "pathdata", "attrs:{d:", " d=\"m", " d='m",
        "fill:", "stroke:", "svgpath", "iconpath",
    )
    if any(marker in context for marker in svg_markers):
        return False, ""
    # Minified SVG coordinates frequently contain values such as l.1.1.1.1c.
    compact = text[max(0, match.start() - 12): min(len(text), match.end() + 12)]
    if re.search(r"[MmLlHhVvCcSsQqTtAaZz][0-9.,+\-]*" + re.escape(value), compact) or re.search(
        re.escape(value) + r"[0-9.,+\-]*[MmLlHhVvCcSsQqTtAaZz]", compact
    ):
        return False, ""
    version_terms = r"(?:@?version|\bver\b|release|jquery|bootstrap|easyui|layui|vue|react|angular|package\.json|sourcemappingurl)"
    if re.search(version_terms + r"[^\n]{0,100}" + re.escape(value), context, re.I) or re.search(
        re.escape(value) + r"[^\n]{0,60}" + version_terms, context, re.I
    ):
        return False, ""
    # Four numeric version components are common in dependency banners. Public
    # IPs are retained only when the surrounding source gives network meaning;
    # private/loopback addresses remain valuable even without a label.
    if not (address.is_private or address.is_loopback or address.is_link_local) and not re.search(
        r"(?:https?://|wss?://|(?:host|hostname|server|proxy|endpoint|listen|connect|remote|origin|address|ip|网关|服务器|地址)\s*[:=])[^\n]{0,80}" + re.escape(value),
        context,
        re.I,
    ):
        return False, ""
    if value in {"0.0.0.0", "1.2.3.4", "255.255.255.255"} or context.rstrip().endswith("rv:" + value):
        return False, ""
    return True, "private" if address.is_private or address.is_loopback or address.is_link_local else "public"


def plausible_contact(kind: str, text: str, match: re.Match[str], value: str) -> bool:
    context = sensitive_context(text, match.start(), match.end()).lower()
    if kind == "email":
        if value.lower().endswith(("@example.com", "@example.org", "@example.net")):
            return False
        return not any(marker in context for marker in (
            "copyright", "license", "licensed", "@preserve", "contributors", "package.json", "npmjs",
        ))
    if kind == "cn_phone":
        return bool(re.search(r"(?:phone|mobile|telephone|tel|contact|手机号|手机|电话|联系方式)", context, re.I))
    if kind == "mac_address":
        return bool(re.search(r"(?:mac(?:address)?|device|hardware|网卡|设备)", context, re.I))
    return True


def extract_sensitive(source: str, text: str) -> list[dict[str, str]]:
    records = []
    for kind, severity, pattern in sensitive_patterns():
        for match in pattern.finditer(text):
            value = match.group(1) if match.lastindex else match.group(0)
            if kind in {"password_assignment", "cloud_access_key", "encryption_key", "bearer_token", "database_password"} and not plausible_secret(value):
                continue
            if kind == "cn_id" and not valid_cn_id(value):
                continue
            if kind == "ip_address":
                valid, scope = plausible_ip(text, match, value)
                if not valid:
                    continue
            else:
                scope = ""
            if kind in {"email", "cn_phone", "mac_address"} and not plausible_contact(kind, text, match, value):
                continue
            records.append({
                "type": kind, "severity": severity, "source": source, "value": value,
                "sha256": hashlib.sha256(value.encode("utf-8", "ignore")).hexdigest(),
                "scope": scope, "evidence": match.group(0)[:180],
                "context": context_window(text, match.start(), match.end()),
            })
    return unique(records, key=lambda item: f"{item['type']}|{item['sha256']}|{item['source']}")


OPPORTUNITY_RULES: list[tuple[str, str, int, re.Pattern[str]]] = [
    ("privilege_surface", "权限与管理面", 90, re.compile(r"admin|permission|privilege|role|member|tenant|组织|管理员|权限|角色|成员|租户", re.I)),
    ("identity_surface", "身份与账户面", 86, re.compile(r"auth|login|logout|register|signup|oauth|token|session|account|profile|登录|注册|认证|令牌|会话|账户", re.I)),
    ("file_surface", "文件处理面", 84, re.compile(r"upload|download|import|export|attachment|document|file|上传|下载|导入|导出|附件|文档|文件", re.I)),
    ("api_contract", "接口契约面", 82, re.compile(r"graphql|graphiql|swagger|openapi|api-doc|接口文档", re.I)),
    ("business_transaction", "业务交易面", 80, re.compile(r"order|invoice|payment|refund|checkout|coupon|balance|订单|发票|支付|退款|优惠|余额", re.I)),
    ("administration", "配置与审计面", 76, re.compile(r"config|setting|system|audit|log|backup|job|task|配置|设置|系统|审计|日志|备份|任务", re.I)),
    ("data_query", "数据查询面", 68, re.compile(r"search|query|report|statistic|analytics|list|detail|搜索|查询|报表|统计|分析|列表|详情", re.I)),
]


def product_signals(fingerprint: dict[str, Any], framework: Any) -> list[str]:
    """Normalize the mixed framework shapes produced by static and runtime probes.

    ``framework_signals`` returns a summary object while older/runtime-only
    inputs may still be a list of records or strings. Opportunity generation
    is a reporting stage, so an unfamiliar shape must never abort the scan.
    """
    values: list[str] = []
    for section in ("backend", "server", "waf", "cdn"):
        value = fingerprint.get(section, {})
        name = str(value.get("name", "") if isinstance(value, dict) else value).strip()
        if name and name.lower() != "unknown":
            values.append(name)

    framework_items: list[Any] = []
    if isinstance(framework, dict):
        primary_name = str(framework.get("framework") or framework.get("name") or "").strip()
        if primary_name and primary_name.lower() != "unknown":
            framework_items.append({
                "name": primary_name,
                "version": framework.get("version", ""),
            })
        for key in ("alternatives", "libraries"):
            records = framework.get(key, [])
            if isinstance(records, list):
                framework_items.extend(records)
        build_tools = framework.get("buildTools", [])
        if isinstance(build_tools, list):
            framework_items.extend(build_tools)
    elif isinstance(framework, list):
        framework_items = framework
    elif framework:
        framework_items = [framework]

    for item in framework_items:
        if isinstance(item, dict):
            name = str(item.get("name") or item.get("framework") or "").strip()
            version = str(item.get("version", "")).strip()
        else:
            name = str(item).strip()
            version = ""
        if name:
            values.append(f"{name} {version}".strip())
    return unique(values)


def build_security_opportunities(
    final_url: str,
    fingerprint: dict[str, Any],
    framework: Any,
    api_candidates: list[dict[str, Any]],
    routes: list[dict[str, Any]],
    runtime: dict[str, Any],
) -> list[dict[str, Any]]:
    """Turn deterministic evidence into a short, explainable investigation inbox."""
    opportunities: list[dict[str, Any]] = []
    runtime_requests = {
        f"{str(item.get('method', 'GET')).upper()}|{item.get('url', '')}": item
        for item in runtime.get("requests", [])
        if isinstance(item, dict)
    }
    for candidate in api_candidates:
        url = str(candidate.get("url") or candidate.get("path") or "").strip()
        if not url:
            continue
        method = str(candidate.get("method", "UNKNOWN") or "UNKNOWN").upper()
        observed = runtime_requests.get(f"{method}|{url}")
        text = " ".join((url, str(candidate.get("evidence", "")), str((observed or {}).get("feature", ""))))
        matches = [(category, label, base) for category, label, base, pattern in OPPORTUNITY_RULES if pattern.search(text)]
        category, label, score = max(matches, key=lambda item: item[2]) if matches else ("api_surface", "接口测试面", 54)
        parameters = unique([
            *[str(value) for value in candidate.get("parameters", [])],
            *[str(value) for value in (observed or {}).get("queryKeys", [])],
            *[str(value) for value in (observed or {}).get("bodyKeys", [])],
        ])
        why = [f"发现 {method} 接口，属于{label}"]
        if observed:
            score += 12
            why.append("浏览器运行时已真实观察到该请求，不只是 JS 字符串命中")
        if method in {"POST", "PUT", "PATCH", "DELETE"}:
            score += 8
            why.append("请求会改变服务端状态，参数与授权边界值得优先验证")
        if parameters:
            score += min(8, len(parameters) * 2)
            why.append(f"已还原 {len(parameters)} 个参数名，可直接构造验证请求")
        status = (observed or {}).get("status")
        if isinstance(status, (int, float)) and 200 <= status < 400:
            score += 4
            why.append(f"运行时响应状态为 {int(status)}")
        observed_headers = (observed or {}).get("effectiveRequestHeaders") or (observed or {}).get("headers") or {}
        header_names = [str(name) for name in observed_headers.keys()] if isinstance(observed_headers, dict) else []
        declared_headers = candidate.get("declaredHeaders", []) if isinstance(candidate.get("declaredHeaders"), list) else []
        custom_header_names = unique([
            *[name for name in header_names if re.search(r"(?:authorization|token|api[-_]?key|tenant|trace|signature|timestamp|nonce|device|client|version)", name, re.I)],
            *[str(item.get("name", "")) for item in declared_headers if isinstance(item, dict)],
        ])
        if custom_header_names:
            score += min(6, 2 + len(custom_header_names))
            why.append(f"已识别请求头契约：{'、'.join(custom_header_names[:6])}")
        score = min(100, score)
        key_source = f"{category}|{method}|{url}"
        opportunities.append({
            "opportunityKey": hashlib.sha256(key_source.encode("utf-8", "ignore")).hexdigest()[:24],
            "targetUrl": final_url,
            "category": category,
            "title": f"{label} · {method} {urlparse(url).path or url}",
            "score": score,
            "status": "ready" if score >= 65 else "queued",
            "confidence": "high" if observed else str(candidate.get("confidence", "medium")),
            "source": "runtime-request" if observed else str(candidate.get("extractionEngine", "frontend-recon")),
            "endpoint": url,
            "method": method,
            "parameters": parameters,
            "whyValuable": why,
            "evidenceRefs": [{
                "type": "runtime-request" if observed else "javascript-extraction",
                "stateId": (observed or {}).get("stateId", ""),
                "actionId": (observed or {}).get("actionId", ""),
                "feature": (observed or {}).get("feature", ""),
                "status": status,
                "contentType": (observed or {}).get("contentType", ""),
                "requestHeaderNames": header_names,
                "extraRequestHeaderNames": (observed or {}).get("extraRequestHeaderNames", []),
                "declaredHeaders": declared_headers,
                "evidence": candidate.get("evidence", ""),
            }],
            "recommendedAction": {
                "type": "deterministic-validation",
                "label": "按原始请求复现并检查参数、对象和权限边界",
                "steps": ["使用运行时捕获的 method、path 与参数名构造请求", "先做响应差异与对象边界检查", "只有出现异常差异时才交给 Strix 语义分析"],
            },
            "requestContext": observed or {},
            "requestHeaders": observed_headers,
            "declaredHeaders": declared_headers,
        })

    signals = product_signals(fingerprint, framework)
    if signals:
        key_source = "product_match|" + "|".join(signals)
        opportunities.append({
            "opportunityKey": hashlib.sha256(key_source.encode("utf-8", "ignore")).hexdigest()[:24],
            "targetUrl": final_url,
            "category": "product_match",
            "title": "产品/框架知识匹配 · " + " / ".join(signals[:4]),
            "score": 64,
            "status": "queued",
            "confidence": "medium",
            "source": "fingerprint",
            "productSignals": signals,
            "whyValuable": ["已经获得可用于匹配本地 Wiki、Skills 与规则包的产品/框架信号"],
            "evidenceRefs": [{"type": "fingerprint", "signals": signals}],
            "recommendedAction": {
                "type": "knowledge-match",
                "label": "匹配本地知识后只运行对应版本/产品的模板 PoC",
                "steps": ["检索本地知识与已安装规则包", "核对产品和版本证据", "仅执行命中的确定性 PoC"],
            },
        })

    high_value_routes = [
        item for item in routes
        if any(pattern.search(str(item.get("path", ""))) for _, _, _, pattern in OPPORTUNITY_RULES)
    ][:20]
    for route in high_value_routes:
        path = str(route.get("path", ""))
        key_source = f"frontend_route|{path}"
        opportunities.append({
            "opportunityKey": hashlib.sha256(key_source.encode("utf-8", "ignore")).hexdigest()[:24],
            "targetUrl": final_url,
            "category": "frontend_feature",
            "title": f"前端功能入口 · {path}",
            "score": 62 + min(12, business_signal_score(path)),
            "status": "ready",
            "confidence": str(route.get("confidence", "medium")),
            "source": str(route.get("extractionEngine", "frontend-route")),
            "route": path,
            "whyValuable": ["前端路由暴露了可直接渲染和触发的业务功能入口"],
            "evidenceRefs": [{"type": "frontend-route", "record": route}],
            "recommendedAction": {"type": "runtime-route", "label": "直接渲染路由并捕获新增 XHR/Fetch", "steps": ["访问路由", "触发标签页、菜单与详情控件", "把新增请求合并到接口清单"]},
        })

    opportunities = unique(opportunities, key=lambda item: item["opportunityKey"])
    opportunities.sort(key=lambda item: int(item.get("score", 0)), reverse=True)
    if not any(int(item.get("score", 0)) >= 70 for item in opportunities):
        key_source = f"fallback_discovery|{urlparse(final_url).netloc}"
        opportunities.append({
            "opportunityKey": hashlib.sha256(key_source.encode("utf-8", "ignore")).hexdigest()[:24],
            "targetUrl": final_url,
            "category": "fallback_discovery",
            "title": "一次性目录/API 兜底发现",
            "score": 40,
            "status": "queued",
            "confidence": "high",
            "source": "stop-policy",
            "whyValuable": ["当前证据没有形成 70 分以上的高价值候选，进入保底流程"],
            "evidenceRefs": [{"type": "coverage", "coverage": runtime.get("coverage", {}), "stopReason": runtime.get("stopReason", "")}],
            "recommendedAction": {
                "type": "bounded-discovery",
                "label": "执行一轮有上限的目录与接口字典发现",
                "steps": ["优先使用指纹和 JS 词汇生成小字典", "单个 401/403 记录为权限边界并继续其他功能；确认 WAF/验证码或持续限流才停止", "本轮无新增高价值证据则结束目标"],
            },
        })
    return opportunities[:80]


def analyze_target(
    target: dict[str, str], timeout: float, max_js_files: int, max_js_bytes: int,
    ast_helper: str, runtime_helper: str, cache: ReconCache, max_api_probes: int,
    auth_session: dict[str, Any] | None = None,
) -> dict[str, Any]:
    original = target.get("url", "").strip()
    if not urlparse(original).scheme:
        original = "https://" + original
    started = time.monotonic()
    result: dict[str, Any] = {"url": original, "company": target.get("company", ""), "errors": []}
    try:
        html_limit = 0 if max_js_bytes <= 0 else min(max_js_bytes, 2_000_000)
        html, headers, status, final_url = fetch(original, timeout, html_limit)
    except Exception as error:
        result["errors"].append(str(error))
        result["durationMs"] = int((time.monotonic() - started) * 1000)
        return result
    parser = PageParser()
    try:
        parser.feed(html)
    except Exception as error:
        result["errors"].append(f"html parse: {error}")
    runtime = run_browser_runtime(final_url, runtime_helper, timeout, auth_session)
    runtime_scripts = [str(value) for value in runtime.get("scripts", []) if str(value).startswith(("http://", "https://"))]
    script_urls = unique([
        urljoin(final_url, value)
        for value in [*parser.scripts, *runtime_scripts]
        if not value.startswith(("data:", "javascript:"))
    ])
    # Put application entry points first so a page with many preload/vendor
    # tags cannot exhaust the analysis budget before main/app/index bundles.
    fetched_scripts: list[tuple[str, str]] = []
    fingerprint_scripts: list[tuple[str, str]] = []
    ast_outputs: list[dict[str, Any]] = []
    source_map_sources: list[tuple[str, str]] = []
    js_records: list[dict[str, Any]] = []
    skipped_urls: set[str] = set()

    def record_skipped(script_url: str, discovered_from: str = "") -> None:
        if script_url in skipped_urls:
            return
        skipped_urls.add(script_url)
        script_type = classify_script(script_url)
        js_records.append({
            "url": script_url, "type": script_type, "statusCode": 0, "size": 0,
            "skipped": True, "discoveredFrom": discovered_from,
            "analysis": {
                "depth": "skipped",
                "reason": "third_party_dependency_or_loader",
            },
        })

    pending: dict[str, str] = {}
    for script_url in script_urls:
        if should_fetch_for_discovery(classify_script(script_url)):
            pending.setdefault(script_url, "html")
        else:
            record_skipped(script_url, "html")
    processed: set[str] = set()
    while pending and (max_js_files <= 0 or len(processed) < max_js_files):
        script_url = max(pending, key=script_priority)
        discovered_from = pending.pop(script_url)
        if script_url in processed:
            continue
        processed.add(script_url)
        script_type = classify_script(script_url)
        try:
            body, js_headers, js_status, js_final = cached_fetch(cache, script_url, timeout, max_js_bytes)
            effective_type = "vendor" if probable_vendor_bundle(js_final, body) else script_type
            ast_result = run_babel_ast(js_final, body, ast_helper, cache) if should_fetch_for_discovery(effective_type) else {}
            fingerprint_scripts.append((js_final, body))
            if should_deep_analyze(effective_type):
                fetched_scripts.append((js_final, body))
                ast_outputs.append(ast_result)
            references = referenced_script_urls(js_final, body)
            references.extend(ast_import_urls(js_final, ast_result.get("imports", [])))
            if should_fetch_for_discovery(effective_type):
                for referenced_url in unique(references):
                    if referenced_url in processed or referenced_url in pending:
                        continue
                    if should_fetch_for_discovery(classify_script(referenced_url)):
                        pending[referenced_url] = js_final
                    else:
                        record_skipped(referenced_url, js_final)
            source_map_match = re.search(r"[#@]\s*sourceMappingURL\s*=\s*([^\s*]+)", body)
            source_map = {}
            if source_map_match and not source_map_match.group(1).startswith("data:"):
                map_url = urljoin(js_final, source_map_match.group(1).strip())
                try:
                    map_limit = 0 if max_js_bytes <= 0 else min(8_000_000, max_js_bytes * 2)
                    map_body, _, map_status, map_final = cached_fetch(cache, map_url, timeout, map_limit)
                    map_json = json.loads(map_body)
                    sources = map_json.get("sources", []) if isinstance(map_json, dict) else []
                    contents = map_json.get("sourcesContent", []) if isinstance(map_json, dict) else []
                    embedded = 0
                    for source_name, source_body in zip(sources, contents):
                        if not isinstance(source_body, str) or not source_body.strip():
                            continue
                        if "node_modules/" in str(source_name).replace("\\", "/"):
                            continue
                        source_map_sources.append((f"{map_final}#{source_name}", source_body))
                        embedded += 1
                    source_map = {"url": map_final, "statusCode": map_status, "sourceCount": len(sources), "embeddedBusinessSources": embedded}
                except Exception as error:
                    source_map = {"url": map_url, "error": str(error)[:300]}
            file_runtime_signals = extract_runtime_signals(js_final, body)
            js_records.append({
                "url": js_final, "type": effective_type, "statusCode": js_status,
                "discoveredFrom": discovered_from,
                "size": len(body.encode("utf-8", "ignore")), "isMinified": body.count("\n") < max(5, len(body) // 5000),
                "contentType": js_headers.get("Content-Type", ""),
                "analysis": {
                    "sourceMapReference": bool(source_map_match),
                    "sourceMap": source_map,
                    "module": "import(" in body or "export{" in body,
                    "moduleCount": ast_result.get("moduleCount", 0),
                    "businessScore": business_signal_score(body),
                    "runtimeSignals": [signal["type"] for signal in file_runtime_signals],
                    "extractionEngine": "babel-ast" if ast_result and ast_result.get("parseErrors") != ["babel_ast_unavailable"] else "regex-fallback",
                    "parseErrors": ast_result.get("parseErrors", []),
                    "depth": "deep" if should_deep_analyze(effective_type) else "discovery",
                    "reason": "business_or_application_bundle" if should_deep_analyze(effective_type) else "loader_manifest_only",
                },
            })
        except Exception as error:
            js_records.append({"url": script_url, "type": classify_script(script_url), "error": str(error), "size": 0})
    inline_sources = [(f"{final_url}#inline-script-{index}", body) for index, body in enumerate(parser.inline_scripts, 1)]
    inline_ast = [run_babel_ast(source, body, ast_helper, cache) for source, body in inline_sources]
    ast_outputs.extend(inline_ast)
    framework = framework_signals(
        html,
        [*fingerprint_scripts, *inline_sources],
        parser.angular_version,
        runtime.get("frameworks", []),
    )
    server = headers.get("Server", "")
    powered = headers.get("X-Powered-By", "")
    fingerprint = {
        "frontend": framework,
        "backend": {"name": powered or "Unknown", "confidence": "medium" if powered else "low"},
        "server": {"name": server or "Unknown", "confidence": "high" if server else "low"},
        "waf": {"name": "Cloudflare" if headers.get("CF-Ray") else "Unknown", "confidence": "high" if headers.get("CF-Ray") else "low"},
        "cdn": {"name": "Cloudflare" if headers.get("CF-Cache-Status") else headers.get("X-Cache", "Unknown"), "confidence": "medium"},
    }
    deep_scripts = [(url, body) for url, body in fetched_scripts if should_deep_analyze(classify_script(url))]
    all_sources = [(final_url, html)] + inline_sources + deep_scripts + source_map_sources
    source_map_ast = [run_babel_ast(source, body, ast_helper, cache) for source, body in source_map_sources]
    ast_outputs.extend(source_map_ast)
    base_urls = unique([
        urljoin(final_url, value)
        for _, body in all_sources
        for value in BASE_URL.findall(body)
    ] + [
        urljoin(final_url, str(value))
        for ast_result in ast_outputs
        for value in ast_result.get("baseUrls", [])
    ])
    apis, routes, sensitive, runtime_signals, crypto_signals = [], [], [], [], []
    for source, body in all_sources:
        apis.extend(extract_apis(final_url, source, body, base_urls))
        apis.extend(normalize_ast_apis(final_url, run_jsluice(source, body), base_urls))
        routes.extend(extract_regex_routes(source, body))
        sensitive.extend(extract_sensitive(source, body))
        runtime_signals.extend(extract_runtime_signals(source, body))
        crypto_signals.extend(extract_crypto_signals(source, body))
    for ast_result in ast_outputs:
        apis.extend(normalize_ast_apis(final_url, ast_result.get("apis", []), base_urls))
        for record in ast_result.get("routes", []):
            item = dict(record)
            item["path"] = normalize_route_path(str(item.get("path") or item.get("rawPath") or ""))
            if valid_frontend_route(item["path"]):
                routes.append(item)
    for record in runtime.get("routes", []):
        item = dict(record)
        item["path"] = normalize_route_path(str(item.get("path") or item.get("rawPath") or ""))
        if valid_frontend_route(item["path"]):
            routes.append(item)
    for request in runtime.get("requests", []):
        resource_type = str(request.get("resourceType", ""))
        if resource_type.lower() not in {"fetch", "xhr", "eventsource"}:
            continue
        value = str(request.get("url", "")).strip()
        method = str(request.get("method", "GET")).upper()
        if not valid_api_path(value, method, "browser-runtime"):
            continue
        observed_parameters = unique([
            *[str(key) for key in request.get("queryKeys", [])],
            *[str(key) for key in request.get("bodyKeys", [])],
        ])
        apis.append({
            "path": urlparse(value).path or value,
            "url": value,
            "method": method,
            "parameters": observed_parameters or [key for key, _ in parse_qsl(urlparse(value).query, keep_blank_values=True)],
            "source": "browser-runtime",
            "confidence": "high",
            "extractionEngine": "browser-runtime",
            "evidence": f"{resource_type} request observed while exploring {request.get('feature') or 'the rendered page'}",
            "statusCode": request.get("status"),
            "contentType": request.get("contentType", ""),
            "stateId": request.get("stateId", ""),
            "actionId": request.get("actionId", ""),
            "feature": request.get("feature", ""),
            "initiator": request.get("initiator", {}),
            "documentUrl": request.get("documentUrl", ""),
            "protocol": request.get("protocol", ""),
            "fromServiceWorker": bool(request.get("fromServiceWorker")),
            "postData": request.get("postData", ""),
            "requestHeaders": request.get("effectiveRequestHeaders") or request.get("headers", {}),
            "requestHeaderNames": request.get("effectiveRequestHeaderNames") or request.get("headerNames", []),
            "extraRequestHeaderNames": request.get("extraRequestHeaderNames", []),
            "extraInfoRequestHeaderNames": request.get("extraInfoRequestHeaderNames", []),
            "associatedCookies": request.get("associatedCookies", []),
            "responseHeaders": request.get("effectiveResponseHeaders") or request.get("responseHeaders", {}),
            "responseHeaderNames": request.get("effectiveResponseHeaderNames") or request.get("responseHeaderNames", []),
            "responseKeys": request.get("responseKeys", []),
            "responsePreview": str(request.get("responsePreview", ""))[:12_000],
        })
    api_intelligence = build_api_intelligence(
        final_url,
        runtime.get("requests", []),
        ast_outputs,
        apis,
    )
    header_intelligence = build_header_intelligence(runtime.get("requests", []), ast_outputs)
    realtime_endpoints = unique([
        {
            "url": str(request.get("url", "")),
            "transport": str(request.get("resourceType", "")),
            "method": str(request.get("method", "GET")).upper(),
            "statusCode": request.get("status"),
            "requestHeaders": request.get("effectiveRequestHeaders") or request.get("headers", {}),
            "extraRequestHeaderNames": request.get("extraRequestHeaderNames", []),
            "responseHeaders": request.get("effectiveResponseHeaders") or request.get("responseHeaders", {}),
            "stateId": request.get("stateId", ""),
            "actionId": request.get("actionId", ""),
            "source": str(request.get("source", "browser-runtime")),
        }
        for request in runtime.get("requests", [])
        if str(request.get("resourceType", "")).lower() in {"websocket", "eventsource"}
        and str(request.get("url", "")).strip()
    ], key=lambda item: f"{item.get('transport')}|{item.get('url')}")
    apis.extend(api_intelligence.get("candidates", []))
    for item in apis:
        parsed_api = urlparse(str(item.get("url") or item.get("path") or ""))
        api_path = parsed_api.path or str(item.get("path", ""))
        if not api_path:
            continue
        split = infer_api_split(api_path)
        item.setdefault("origin", f"{parsed_api.scheme}://{parsed_api.netloc}" if parsed_api.scheme and parsed_api.netloc else "")
        item.setdefault("apiPrefix", split["apiPrefix"])
        item.setdefault("businessEndpoint", split["businessEndpoint"])
        item.setdefault("splitReason", split["splitReason"])
        item.setdefault("normalizedPath", normalized_endpoint_path(api_path))
    engine_rank = {"browser-runtime": 6, "babel-ast": 5, "babel-ast-xhr": 5, "evidence-reconstruction": 4, "jsluice-tree-sitter": 3, "regex-fallback": 2, "string-heuristic": 1}
    apis.sort(key=lambda item: (engine_rank.get(item.get("extractionEngine", ""), 0), item.get("confidence") == "high"), reverse=True)
    apis = unique(apis, key=lambda item: f"{item['method']}|{item['url']}")
    apis.sort(key=lambda item: (business_signal_score(item.get("url", "")), item.get("method") in {"POST", "PUT", "PATCH", "DELETE"}, engine_rank.get(item.get("extractionEngine", ""), 0)), reverse=True)
    api_candidates = unique(apis, key=lambda item: f"{item.get('method', 'UNKNOWN')}|{item.get('url', '')}")
    verified_apis, unresolved_apis = verify_api_candidates(
        api_candidates, final_url, html, timeout, max_api_probes,
    )
    mark_registration(verified_apis, ("url", "path"))
    mark_registration(unresolved_apis, ("url", "path"))
    # Registration candidates remain visible to the existing Strix evidence path even
    # when a safe GET cannot verify a POST-only endpoint. No form or POST is submitted.
    registration_candidates = [
        {**item, "candidateOnly": True}
        for item in unresolved_apis
        if item.get("registration", {}).get("category") == "account_registration"
    ]
    reconstructed_candidates = [
        {**item, "candidateOnly": True}
        for item in unresolved_apis
        if item.get("extractionEngine") == "evidence-reconstruction"
        and float(item.get("reconstructionConfidence", 0)) >= 0.72
    ][:20]
    apis = unique([*verified_apis, *registration_candidates, *reconstructed_candidates], key=lambda item: f"{item.get('method', 'UNKNOWN')}|{item.get('url', '')}")
    routes = [item for item in routes if valid_frontend_route(str(item.get("path", "")))]
    route_engine_rank = {
        "browser-runtime": 5,
        "babel-ast": 4,
        "babel-ast-jsx": 4,
        "route-structure-fallback": 2,
    }
    routes.sort(
        key=lambda item: (
            item.get("confidence") == "high",
            route_engine_rank.get(str(item.get("extractionEngine", "")), 0),
        ),
        reverse=True,
    )
    routes = unique(routes, key=lambda item: item["path"])
    routes.sort(key=lambda item: business_signal_score(item.get("path", "")), reverse=True)
    mark_registration(routes, ("path",))
    sensitive = unique(sensitive, key=lambda item: f"{item['type']}|{item['sha256']}")
    sensitive.sort(key=lambda item: item.get("severity") == "high", reverse=True)
    runtime_signals = unique(runtime_signals, key=lambda item: f"{item['type']}|{item['source']}")
    crypto_signals = unique(crypto_signals, key=lambda item: f"{item['category']}|{item['algorithm']}|{item['operation']}|{item['source']}")
    runtime_hook_plan = select_runtime_hook(runtime_signals, apis, routes)
    if runtime_hook_plan:
        runtime_signals.insert(0, {
            "type": "runtime_hook_plan",
            "label": "Single runtime Hook recommendation",
            "source": runtime_hook_plan["source"],
            "hook": runtime_hook_plan["hook"],
            "reason": runtime_hook_plan["reason"],
            "context": runtime_hook_plan["reason"],
        })
    ai_fallback = ai_fallback_evidence(
        framework, deep_scripts + source_map_sources, apis, ast_outputs,
    )
    discovered_links = unique([
        urljoin(final_url, value)
        for value in [*parser.links, *runtime.get("links", [])]
        if not str(value).startswith(("javascript:", "mailto:", "tel:"))
    ])
    registration_links: list[Any] = [
        *discovered_links,
        *parser.link_records,
        *runtime.get("linkRecords", []),
    ]
    discovered_forms = [*parser.forms, *runtime.get("forms", [])]
    registration_records = registration_entrypoints(
        final_url,
        apis,
        unresolved_apis,
        routes,
        registration_links,
        discovered_forms,
        runtime.get("requests", []),
    )
    opportunities = build_security_opportunities(
        final_url,
        fingerprint,
        framework,
        api_candidates,
        routes,
        runtime,
    )
    result.update({
        "finalUrl": final_url, "statusCode": status, "fingerprint": fingerprint,
        "techStack": {"framework": framework, "server": server, "poweredBy": powered, "baseUrls": base_urls},
        "jsFiles": js_records, "apis": apis,
        "apiCandidates": unresolved_apis,
        "apiIntelligence": api_intelligence,
        "headerIntelligence": header_intelligence,
        "realtimeEndpoints": realtime_endpoints,
        "routes": routes,
        "registrationEntrypoints": registration_records,
        "features": runtime.get("features", []),
        "opportunities": opportunities,
        "runtimeExploration": {
            "available": runtime.get("available", False),
            "browser": runtime.get("browser", ""),
            "states": runtime.get("states", []),
            "actions": runtime.get("actions", []),
            "requests": runtime.get("requests", []),
            "blockedRequests": runtime.get("blockedRequests", []),
            "coverage": runtime.get("coverage", {}),
            "stopReason": runtime.get("stopReason", ""),
            "durationMs": runtime.get("durationMs", 0),
            "errors": runtime.get("errors", []),
        },
        "authSessionValidation": runtime.get("authSessionValidation", {
            "applied": False, "valid": False, "clearSessionInvalid": False, "wafDetected": False,
        }),
        "sensitiveInfo": sensitive,
        "runtimeSignals": runtime_signals,
        "cryptoSignals": crypto_signals,
        "aiFallback": ai_fallback,
        "runtimeHookRecommended": bool(runtime_hook_plan),
        "runtimeHookPlan": runtime_hook_plan,
        "analysisSummary": {
            "engine": "babel-ast+deterministic-fallback",
            "jsluice": bool(shutil.which("jsluice")),
            "astScripts": len(ast_outputs),
            "sourceMapBusinessSources": len(source_map_sources),
            "runtimeBrowserAvailable": bool(runtime.get("available")),
            "runtimeBrowser": runtime.get("browser", ""),
            "runtimeRoutes": len(runtime.get("routes", [])),
            "runtimeRequests": len(runtime.get("requests", [])),
            "runtimeStates": len(runtime.get("states", [])),
            "runtimeActions": len(runtime.get("actions", [])),
            "runtimeBlockedMutations": len(runtime.get("blockedRequests", [])),
            "runtimeStopReason": runtime.get("stopReason", ""),
            "registrationEntrypoints": len(registration_records),
            "opportunities": len(opportunities),
            "runtimeErrors": runtime.get("errors", []),
            "cachedScripts": len(cache.responses),
            "apiCandidates": len(api_candidates),
            "verifiedApis": len(verified_apis),
            "unresolvedApis": len(unresolved_apis),
            "apiProbeBudget": max_api_probes,
            "apiClients": len(api_intelligence.get("clients", [])),
            "apiReconstructions": len(api_intelligence.get("reconstructions", [])),
            "reconstructedCandidates": len(api_intelligence.get("candidates", [])),
            "observedRequestHeaders": header_intelligence.get("summary", {}).get("observedHeaderCount", 0),
            "declaredRequestHeaders": header_intelligence.get("summary", {}).get("declaredOnlyHeaderCount", 0),
            "extraInfoRequestHeaders": header_intelligence.get("summary", {}).get("extraInfoHeaderCount", 0),
            "realtimeEndpoints": len(realtime_endpoints),
        },
        "externalScripts": [url for url in script_urls if urlparse(url).netloc != urlparse(final_url).netloc],
        "metaTags": parser.meta,
        "links": discovered_links,
        "forms": discovered_forms,
        "durationMs": int((time.monotonic() - started) * 1000),
    })
    return result


def auth_identity_key(session: dict[str, Any] | None, index: int) -> str:
    if not session:
        return "anonymous"
    session_id = str(session.get("id", "")).strip() or f"identity-{index + 1}"
    name = str(session.get("name", "")).strip()
    return f"session:{session_id}:{name}" if name else f"session:{session_id}"


def tag_identity_run(result: dict[str, Any], identity_key: str, index: int) -> dict[str, Any]:
    """Namespace runtime causality IDs and tag observations for one identity."""
    prefix = f"identity-{index + 1}"
    exploration = result.get("runtimeExploration", {})
    state_map = {
        str(item.get("id", "")): f"{prefix}:{item.get('id', '')}"
        for item in exploration.get("states", [])
        if item.get("id")
    }
    action_map = {
        str(item.get("id", "")): f"{prefix}:{item.get('id', '')}"
        for item in exploration.get("actions", [])
        if item.get("id")
    }
    for state in exploration.get("states", []):
        state["id"] = state_map.get(str(state.get("id", "")), str(state.get("id", "")))
        state["identityKey"] = identity_key
    for action in exploration.get("actions", []):
        action["id"] = action_map.get(str(action.get("id", "")), str(action.get("id", "")))
        action["stateId"] = state_map.get(str(action.get("stateId", "")), str(action.get("stateId", "")))
        action["identityKey"] = identity_key
    for request in exploration.get("requests", []):
        request["stateId"] = state_map.get(str(request.get("stateId", "")), str(request.get("stateId", "")))
        request["actionId"] = action_map.get(str(request.get("actionId", "")), str(request.get("actionId", "")))
        request["identityKey"] = identity_key
        request["identityKeys"] = [identity_key]
    for feature in result.get("features", []):
        feature["stateId"] = state_map.get(str(feature.get("stateId", "")), str(feature.get("stateId", "")))
        feature["identityKey"] = identity_key
    for api in [*result.get("apis", []), *result.get("apiCandidates", [])]:
        api["stateId"] = state_map.get(str(api.get("stateId", "")), str(api.get("stateId", "")))
        api["actionId"] = action_map.get(str(api.get("actionId", "")), str(api.get("actionId", "")))
        api["identityKey"] = identity_key
        api["identityKeys"] = [identity_key]
    for opportunity in result.get("opportunities", []):
        opportunity["identityKey"] = identity_key
        opportunity["identityKeys"] = [identity_key]
        opportunity["stateId"] = state_map.get(str(opportunity.get("stateId", "")), str(opportunity.get("stateId", "")))
        opportunity["actionId"] = action_map.get(str(opportunity.get("actionId", "")), str(opportunity.get("actionId", "")))
    result["identityKey"] = identity_key
    return result


def merge_identity_runs(runs: list[dict[str, Any]]) -> dict[str, Any]:
    if not runs:
        return {}
    base = runs[0]
    api_map: dict[str, dict[str, Any]] = {}
    opportunity_map: dict[str, dict[str, Any]] = {}
    merged_states: list[dict[str, Any]] = []
    merged_actions: list[dict[str, Any]] = []
    merged_requests: list[dict[str, Any]] = []
    merged_blocked: list[dict[str, Any]] = []
    identity_summaries: list[dict[str, Any]] = []
    api_matrix: dict[str, dict[str, Any]] = {}
    coverage_totals: dict[str, int] = {}
    invalid_identities: list[str] = []
    any_waf = False

    for run in runs:
        identity_key = str(run.get("identityKey", "anonymous"))
        exploration = run.get("runtimeExploration", {})
        validation = run.get("authSessionValidation", {})
        any_waf = any_waf or bool(validation.get("wafDetected"))
        if validation.get("clearSessionInvalid"):
            invalid_identities.append(identity_key)
        identity_summaries.append({
            "identityKey": identity_key,
            "finalUrl": run.get("finalUrl", run.get("url", "")),
            "statusCode": run.get("statusCode"),
            "valid": bool(validation.get("valid")),
            "validationReason": validation.get("reason", ""),
            "stateCount": len(exploration.get("states", [])),
            "actionCount": len(exploration.get("actions", [])),
            "apiCount": len(run.get("apis", [])),
        })
        merged_states.extend(exploration.get("states", []))
        merged_actions.extend(exploration.get("actions", []))
        merged_requests.extend(exploration.get("requests", []))
        merged_blocked.extend(exploration.get("blockedRequests", []))
        for key, value in exploration.get("coverage", {}).items():
            if isinstance(value, (int, float)):
                coverage_totals[key] = coverage_totals.get(key, 0) + int(value)
        for api in run.get("apis", []):
            method = str(api.get("method", "GET")).upper()
            url = str(api.get("url") or api.get("path") or "")
            key = f"{method}|{url}"
            matrix = api_matrix.setdefault(key, {})
            matrix[identity_key] = {
                "observed": True,
                "status": api.get("statusCode", api.get("status")),
                "responseKeys": sorted(str(value) for value in api.get("responseKeys", [])),
                "contentType": api.get("contentType", ""),
                "parameters": sorted(str(value) for value in api.get("parameters", [])),
            }
            if key not in api_map:
                api_map[key] = api
                api_map[key]["identityKeys"] = [identity_key]
                api_map[key]["identityObservations"] = [{"identityKey": identity_key, **matrix[identity_key]}]
            else:
                current = api_map[key]
                current["identityKeys"] = sorted(set([*current.get("identityKeys", []), identity_key]))
                current.setdefault("identityObservations", []).append({"identityKey": identity_key, **matrix[identity_key]})
                current["parameters"] = sorted(set([*current.get("parameters", []), *api.get("parameters", [])]))
        for opportunity in run.get("opportunities", []):
            key = str(opportunity.get("opportunityKey") or f"{opportunity.get('category')}|{opportunity.get('title')}")
            if key not in opportunity_map:
                opportunity_map[key] = opportunity
            else:
                current = opportunity_map[key]
                current["identityKeys"] = sorted(set([*current.get("identityKeys", []), identity_key]))
                current["score"] = max(int(current.get("score", 0)), int(opportunity.get("score", 0)))

    all_identities = [item["identityKey"] for item in identity_summaries]
    comparisons: list[dict[str, Any]] = []
    all_api_keys = set(api_matrix)
    for api_key in sorted(all_api_keys):
        observations = api_matrix[api_key]
        for identity in all_identities:
            observations.setdefault(identity, {"observed": False, "status": None, "responseKeys": [], "contentType": "", "parameters": []})
        values = list(observations.values())
        observed_values = [value for value in values if value.get("observed")]
        if len(observed_values) != len(values):
            comparisons.append({"apiKey": api_key, "differenceType": "reachability", "riskScore": 55, "matrix": observations})
        statuses = {str(value.get("status")) for value in observed_values}
        if len(statuses) > 1:
            comparisons.append({"apiKey": api_key, "differenceType": "status", "riskScore": 60, "matrix": observations})
        schemas = {json.dumps(value.get("responseKeys", []), sort_keys=True) for value in observed_values}
        if len(schemas) > 1:
            comparisons.append({"apiKey": api_key, "differenceType": "response_schema", "riskScore": 70, "matrix": observations})

    base["apis"] = list(api_map.values())
    base["opportunities"] = sorted(opportunity_map.values(), key=lambda item: int(item.get("score", 0)), reverse=True)
    base["runtimeExploration"] = {
        **base.get("runtimeExploration", {}),
        "states": merged_states,
        "actions": merged_actions,
        "requests": merged_requests,
        "blockedRequests": merged_blocked,
        "coverage": coverage_totals,
        "stopReason": "confirmed_waf_or_challenge" if any_waf else "identity_matrix_complete",
    }
    base["identityRuns"] = identity_summaries
    base["identityComparisons"] = comparisons
    base["identityMatrix"] = {"identities": all_identities, "apis": api_matrix}
    auth_applied = any(identity != "anonymous" for identity in all_identities)
    base["authSessionValidation"] = {
        "applied": auth_applied,
        "valid": auth_applied and not invalid_identities,
        "clearSessionInvalid": len(invalid_identities) == len(all_identities),
        "invalidIdentityKeys": invalid_identities,
        "wafDetected": any_waf,
        "reason": "confirmed_waf_or_challenge" if any_waf else "identity_matrix_complete",
    }
    base.setdefault("analysisSummary", {})["identityCount"] = len(all_identities)
    base["analysisSummary"]["identityComparisonCount"] = len(comparisons)
    return base


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--targets", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--timeout", type=float, default=15.0)
    parser.add_argument("--max-js-files", type=int, default=0, help="0 means no file-count limit")
    parser.add_argument("--max-js-bytes", type=int, default=0, help="0 means no response truncation")
    parser.add_argument("--max-api-probes", type=int, default=0, help="0 means probe every extracted candidate with safe GET")
    parser.add_argument("--ast-helper", default=str(Path(__file__).with_name("8_js_ast_analyzer.cjs")))
    parser.add_argument("--runtime-helper", default=str(Path(__file__).with_name("9_frontend_runtime_probe.cjs")))
    parser.add_argument("--auth-session", default="")
    args = parser.parse_args()
    targets = json.loads(Path(args.targets).read_text(encoding="utf-8"))
    if isinstance(targets, dict):
        targets = targets.get("targets", [])
    auth_session: dict[str, Any] | None = None
    auth_sessions: list[dict[str, Any] | None] = []
    if args.auth_session:
        try:
            loaded_auth = json.loads(Path(args.auth_session).read_text(encoding="utf-8"))
            if isinstance(loaded_auth, dict):
                if isinstance(loaded_auth.get("sessions"), list):
                    auth_sessions = [value for value in loaded_auth["sessions"] if isinstance(value, dict)]
                else:
                    auth_session = loaded_auth
        except (OSError, ValueError) as error:
            raise RuntimeError(f"无法读取浏览器登录会话：{error}") from error
    output = {
        "schemaVersion": 2,
        "analysisPipeline": ["bounded-browser-exploration", "interaction-request-correlation", "runtime-request-capture", "business-script-classification", "babel-ast", "constant-propagation", "javascript-string-evidence", "api-prefix-splitting", "evidence-backed-endpoint-reconstruction", "request-shape-extraction", "endpoint-resolution", "safe-api-verification", "opportunity-ranking", "sensitive-semantic-filter", "local-crypto-classification", "deterministic-fallback"],
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "sensitiveDataNotice": "Potential values and 200-character source context are stored locally for authorized manual review.",
        "targets": [],
    }
    cache = ReconCache()
    for index, target in enumerate(targets, 1):
        print(f"[{index}/{len(targets)}] frontend recon {target.get('url', '')}", flush=True)
        sessions = auth_sessions or [auth_session]
        identity_runs = []
        for identity_index, session in enumerate(sessions):
            identity = auth_identity_key(session, identity_index)
            print(f"  identity {identity_index + 1}/{len(sessions)}: {identity}", flush=True)
            identity_runs.append(tag_identity_run(analyze_target(
                target, args.timeout, args.max_js_files, args.max_js_bytes,
                args.ast_helper, args.runtime_helper, cache, args.max_api_probes, session,
            ), identity, identity_index))
            if identity_runs[-1].get("authSessionValidation", {}).get("wafDetected"):
                break
        output["targets"].append(merge_identity_runs(identity_runs))
        Path(args.output).write_text(json.dumps(output, ensure_ascii=False, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
