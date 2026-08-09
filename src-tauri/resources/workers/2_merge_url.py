#!/usr/bin/env python3
"""从 FOFA 候选/人工复核表生成规范化 Web URL 清单。"""

from __future__ import annotations

import argparse
import ipaddress
import re
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit, urlunsplit

import pandas as pd


BASE_DIR = Path(__file__).resolve().parent
CONFIRMED_VALUES = {"confirmed", "confirm", "yes", "y", "true", "1", "确认", "已确认", "是"}
PENDING_VALUES = {"", "uncertain", "pending", "待确认", "存疑"}
HTTPS_PORTS = {443, 4443, 6443, 7443, 8443, 9443, 10443}
ILLEGAL_XML_RE = re.compile(r"[\x00-\x08\x0B\x0C\x0E-\x1F]")


def clean(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, dict):
        return str(value)
    if isinstance(value, (list, tuple, set)):
        return " | ".join(clean(item) for item in value if clean(item))
    if not pd.api.types.is_scalar(value):
        try:
            return clean(value.tolist())
        except AttributeError:
            return str(value).strip()
    if pd.isna(value):
        return ""
    return ILLEGAL_XML_RE.sub("", str(value)).strip()


def read_table(path: Path) -> pd.DataFrame:
    if path.suffix.lower() in {".xlsx", ".xls"}:
        return pd.read_excel(path, dtype=str).fillna("")
    return pd.read_csv(path, dtype=str, encoding="utf-8-sig").fillna("")


def parse_port(value: Any) -> int | None:
    value = clean(value)
    if not value:
        return None
    try:
        port = int(float(value))
    except ValueError:
        return None
    return port if 1 <= port <= 65535 else None


def bracket_ipv6(host: str) -> str:
    host = host.strip("[]")
    try:
        return f"[{host}]" if ipaddress.ip_address(host).version == 6 else host
    except ValueError:
        return host.lower().rstrip(".")


def infer_scheme(raw_scheme: str, protocol: str, port: int | None) -> str:
    if raw_scheme.lower() in {"http", "https"}:
        return raw_scheme.lower()
    protocol = protocol.lower()
    if "https" in protocol or "ssl" in protocol or "tls" in protocol:
        return "https"
    if port in HTTPS_PORTS:
        return "https"
    return "http"


def build_url(row: dict[str, Any]) -> str | None:
    """优先使用 FOFA link/host，必要时回退到 IP，并规范协议和端口。"""
    raw = clean(row.get("link")) or clean(row.get("host"))
    ip = clean(row.get("ip"))
    protocol = clean(row.get("protocol"))
    port = parse_port(row.get("port"))

    if not raw:
        raw = ip
    if not raw:
        return None

    # 排除明显的非 Web 协议；FOFA host/link 自带 http(s) 时仍允许。
    raw_has_http = bool(re.match(r"^https?://", raw, flags=re.I))
    if protocol and "http" not in protocol.lower() and not raw_has_http:
        return None

    if "://" not in raw:
        try:
            if ipaddress.ip_address(raw).version == 6:
                raw = f"[{raw}]"
        except ValueError:
            pass
    parsed = urlsplit(raw if "://" in raw else f"//{raw}")
    host = parsed.hostname or ""
    if not host:
        return None

    try:
        parsed_port = parsed.port
    except ValueError:
        parsed_port = None
    scheme = infer_scheme(parsed.scheme, protocol, parsed_port or port)
    effective_port = parsed_port or port
    host_for_url = bracket_ipv6(host)

    # 默认端口不重复输出；非默认端口保留。
    if effective_port and not (
        (scheme == "http" and effective_port == 80)
        or (scheme == "https" and effective_port == 443)
    ):
        netloc = f"{host_for_url}:{effective_port}"
    else:
        netloc = host_for_url

    path = parsed.path or ""
    # 资产清单不保留 query/fragment，避免同一入口因参数重复。
    return urlunsplit((scheme, netloc, path, "", ""))


def canonical_url(url: str) -> str:
    parsed = urlsplit(url)
    path = re.sub(r"/{2,}", "/", parsed.path or "")
    if path == "/":
        path = ""
    return urlunsplit((parsed.scheme.lower(), parsed.netloc.lower(), path.rstrip("/"), "", ""))


def decision_group(row: dict[str, Any], has_decision: bool, min_score: int) -> str:
    if has_decision:
        decision = clean(row.get("decision")).lower()
        if decision in CONFIRMED_VALUES:
            return "confirmed"
        if decision in PENDING_VALUES:
            return "pending"
        return "rejected"

    try:
        score = int(float(clean(row.get("score")) or 0))
    except ValueError:
        score = 0
    return "confirmed" if score >= min_score else "pending"


def write_lines(path: Path, values: list[str]) -> None:
    path.write_text("\n".join(values) + ("\n" if values else ""), encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="合并 FOFA 候选并生成确认/待确认 URL 清单")
    parser.add_argument("--input", type=Path, default=BASE_DIR / "output" / "manual_review.xlsx")
    parser.add_argument("--output-dir", type=Path, default=BASE_DIR / "output")
    parser.add_argument("--min-score", type=int, default=85, help="输入无 decision 列时的自动确认阈值")
    parser.add_argument("--include-all", action="store_true", help="兼容旧流程：final_urls.txt 包含确认和待确认项")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.input.exists():
        print(f"[-] 输入文件不存在：{args.input}")
        return 2

    args.output_dir.mkdir(parents=True, exist_ok=True)
    frame = read_table(args.input)
    if frame.empty:
        print("[-] 输入表为空")
        return 2

    # 兼容旧表前三列：host/ip/port。新流程始终优先按列名读取。
    required_source_columns = {"host", "link", "ip"}
    if not required_source_columns.intersection(frame.columns):
        legacy = pd.read_excel(args.input, engine="openpyxl", header=None).fillna("")
        frame = pd.DataFrame({
            "host": legacy.iloc[:, 0],
            "ip": legacy.iloc[:, 1] if legacy.shape[1] > 1 else "",
            "port": legacy.iloc[:, 2] if legacy.shape[1] > 2 else "",
            "protocol": "",
            "score": 100,
        })

    has_decision = "decision" in frame.columns
    records: dict[tuple[str, str], dict[str, Any]] = {}

    for row in frame.to_dict("records"):
        url = build_url(row)
        if not url:
            continue
        url = canonical_url(url)
        company = clean(row.get("company"))
        key = (company, url)
        group = decision_group(row, has_decision, args.min_score)
        record = records.setdefault(key, {
            "company": company,
            "url": url,
            "decision_group": group,
            "score": clean(row.get("score")),
            "confidence": clean(row.get("confidence")),
            "evidence": clean(row.get("evidence")),
            "ip": clean(row.get("ip")),
            "port": clean(row.get("port")),
            "title": clean(row.get("title")),
        })
        # 同企业同 URL 多次出现时，confirmed > pending > rejected。
        priority = {"rejected": 0, "pending": 1, "confirmed": 2}
        if priority[group] > priority[record["decision_group"]]:
            record["decision_group"] = group

    result = pd.DataFrame(records.values())
    if result.empty:
        print("[-] 没有可生成的 Web URL")
        return 2

    result = result.sort_values(["decision_group", "company", "url"])
    result.to_csv(args.output_dir / "url_inventory.csv", index=False, encoding="utf-8-sig")

    confirmed = sorted(set(result.loc[result["decision_group"] == "confirmed", "url"]))
    pending = sorted(set(result.loc[result["decision_group"] == "pending", "url"]))
    rejected = sorted(set(result.loc[result["decision_group"] == "rejected", "url"]))

    write_lines(args.output_dir / "confirmed_urls.txt", confirmed)
    write_lines(args.output_dir / "pending_urls.txt", pending)
    write_lines(args.output_dir / "rejected_urls.txt", rejected)
    final_urls = sorted(set(confirmed + pending)) if args.include_all else confirmed
    write_lines(args.output_dir / "final_urls.txt", final_urls)

    print(f"[+] 确认 URL：{len(confirmed)}")
    print(f"[+] 待确认 URL：{len(pending)}")
    print(f"[+] 排除 URL：{len(rejected)}")
    print(f"[+] final_urls.txt：{len(final_urls)}（{'确认+待确认' if args.include_all else '仅确认'}）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
