"""Conservative offline translation of explicit HTTP fingerprint query literals.

This is not a FOFA/Goby query engine. It preserves complete supported boolean
structure, literal byte casing, and literal-vs-regex distinctions. Local raw HTML
<title> matching cannot reproduce a search engine's HTML entity decoding, title
normalization or historical index. Unknown dialect semantics are rejected rather
than guessed. No source script is evaluated and no requests are made.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
import re
from typing import Any

MAX_QUERY_BYTES = 32768
MAX_LITERAL_BYTES = 4096
MAX_TOKENS = 2048
MAX_DEPTH = 32


class ConversionError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


def _fail(code: str, message: str) -> None:
    raise ConversionError(code, message)


def _quoted(text: str, start: int) -> tuple[str, int]:
    quote = text[start]
    chars: list[str] = []
    pos = start + 1
    while pos < len(text):
        char = text[pos]
        if char == quote:
            return "".join(chars), pos + 1
        if char == "\\":
            if pos + 1 >= len(text):
                _fail("unterminated_string", "字符串以不完整转义结尾")
            following = text[pos + 1]
            # Decode just quote/backslash escapes in the actual DSL string. JSON
            # envelope decoding happens separately and exactly once per wrapper.
            if following in (quote, "\\"):
                chars.append(following)
            else:
                chars.extend((char, following))
            pos += 2
            continue
        chars.append(char)
        pos += 1
    _fail("unterminated_string", "查询字符串缺少结束引号")


def _unique_json_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            _fail("duplicate_json_key", "查询 JSON 包装包含重复字段，不能选择性覆盖")
        result[key] = value
    return result


def normalize_query(query: str) -> str:
    """Remove only whole, explicitly recognized presentation envelopes.

    Plain escaped-quote fragments are deliberately not globally unescaped. A
    valid whole JSON string or one-key GobyQuery/FOFAQuery object is unwrapped by
    json.loads; arbitrary prose, HTML wrappers and partial JSON are never cut out.
    """
    if not isinstance(query, str):
        _fail("invalid_query_type", "查询必须是字符串")
    if len(query.encode("utf-8")) > MAX_QUERY_BYTES:
        _fail("query_too_long", "查询超过 32768 字节")
    text = query.strip()
    if not text:
        _fail("empty_query", "查询为空")
    for _ in range(12):
        previous = text
        if text.startswith("{") or text.startswith('"'):
            try:
                decoded = json.loads(text, object_pairs_hook=_unique_json_object)
            except ConversionError:
                raise
            except (ValueError, TypeError):
                decoded = None
            if isinstance(decoded, str):
                text = decoded.strip()
            elif isinstance(decoded, dict):
                if len(decoded) != 1 or next(iter(decoded)) not in {"GobyQuery", "FOFAQuery", "FofaQuery"}:
                    _fail("unsupported_json_wrapper", "仅接受单一 GobyQuery/FOFAQuery 字符串包装")
                value = next(iter(decoded.values()))
                if not isinstance(value, str):
                    _fail("unsupported_json_wrapper", "查询包装中的值必须是字符串")
                text = value.strip()
        if text.startswith("```"):
            if not text.endswith("```") or len(text) < 6:
                _fail("malformed_wrapper", "代码围栏不完整")
            body = text[3:-3]
            if "\n" in body:
                first, remainder = body.split("\n", 1)
                if first.strip().lower() in {"", "fofa", "goby", "text", "plaintext"}:
                    text = remainder.strip()
                else:
                    _fail("unsupported_wrapper", "未知代码围栏语言")
            else:
                text = body.strip()
        elif text.startswith("`"):
            if not text.endswith("`") or len(text) < 2:
                _fail("malformed_wrapper", "行内代码包装不完整")
            text = text[1:-1].strip()
        if text.startswith(">"):
            text = text[1:].lstrip()
        if re.match(r"^[+*-]\s+", text):
            text = re.sub(r"^[+*-]\s+", "", text, count=1)
        prefix = re.match(r"(?i)^(?:fofa-query|fofaquery|gobyquery|fofa|goby)(?=\s|[:：`'\"])(?:\s*[:：])?\s*", text)
        if prefix:
            text = text[prefix.end():].strip()
        if text[:1] in {"'", '"'}:
            # A complete quoted query is a presentation wrapper, not a license
            # to discard unmatched leading/trailing text or change inner escapes.
            try:
                decoded, end = _quoted(text, 0)
            except ConversionError:
                decoded, end = "", -1
            if end == len(text):
                text = decoded.strip()
        if not text:
            _fail("empty_query", "查询包装中没有查询内容")
        if text == previous:
            return text
    _fail("wrapper_depth", "查询包装层数过多")


@dataclass(frozen=True)
class _Token:
    kind: str
    value: str
    position: int


def _tokens(text: str) -> list[_Token]:
    result: list[_Token] = []
    pos = 0
    while pos < len(text):
        if text[pos].isspace():
            pos += 1
            continue
        if text[pos:pos + 2] in {"==", "!=", "<=", ">=", "*=", "~=", "=~"}:
            _fail("unsupported_comparison", f"不支持比较运算符 {text[pos:pos + 2]}")
        if text[pos:pos + 2] in {"&&", "||"}:
            result.append(_Token(text[pos:pos + 2], text[pos:pos + 2], pos))
            pos += 2
        elif text[pos] in "()=":
            result.append(_Token(text[pos], text[pos], pos))
            pos += 1
        elif text[pos] in "'\"":
            value, end = _quoted(text, pos)
            result.append(_Token("string", value, pos))
            pos = end
        elif text[pos] in "!&|":
            _fail("unsupported_operator", "只接受文档明确的 && 与 ||，不接受取反或单字符运算符")
        else:
            match = re.match(r"[A-Za-z_][A-Za-z0-9_.-]*", text[pos:])
            if not match:
                _fail("unexpected_token", f"查询位置 {pos} 存在无法解析的内容")
            value = match.group(0)
            if value.upper() in {"AND", "OR", "NOT"}:
                _fail("unsupported_operator", "不推断文本逻辑运算符的方言语义")
            result.append(_Token("field", value, pos))
            pos += len(value)
        if len(result) > MAX_TOKENS:
            _fail("query_too_complex", "查询令牌数过多")
    result.append(_Token("end", "", len(text)))
    return result


class _Parser:
    def __init__(self, text: str):
        self.tokens = _tokens(text)
        self.index = 0

    def _is(self, kind: str) -> bool:
        return self.tokens[self.index].kind == kind

    def _take(self, kind: str) -> _Token:
        token = self.tokens[self.index]
        if token.kind != kind:
            _fail("incomplete_expression", f"位置 {token.position} 需要 {kind}，不能丢弃多余文本")
        self.index += 1
        return token

    def parse(self) -> tuple:
        value = self._or(0)
        self._take("end")
        return value

    def _or(self, depth: int) -> tuple:
        items = [self._and(depth)]
        while self._is("||"):
            self._take("||")
            items.append(self._and(depth))
        return items[0] if len(items) == 1 else ("or", *items)

    def _and(self, depth: int) -> tuple:
        items = [self._primary(depth)]
        while self._is("&&"):
            self._take("&&")
            items.append(self._primary(depth))
        return items[0] if len(items) == 1 else ("and", *items)

    def _primary(self, depth: int) -> tuple:
        if depth > MAX_DEPTH:
            _fail("query_too_complex", "查询括号嵌套过深")
        if self._is("("):
            self._take("(")
            value = self._or(depth + 1)
            self._take(")")
            return value
        name = self._take("field").value
        self._take("=")
        value = self._take("string").value
        return ("predicate", name, value)


def _simplify(node: tuple) -> tuple:
    if node[0] == "predicate":
        return node
    children: list[tuple] = []
    for child in node[1:]:
        child = _simplify(child)
        children.extend(child[1:] if child[0] == node[0] else [child])
    unique = sorted(set(children), key=repr)
    return unique[0] if len(unique) == 1 else (node[0], *unique)


def _literal(value: str) -> str:
    if not value.strip() or not any(character.isalnum() for character in value):
        _fail("generic_or_empty_literal", "没有可用的非空产品字面特征")
    if len(value.encode("utf-8")) > MAX_LITERAL_BYTES:
        _fail("literal_too_long", "字面特征超过 4096 字节")
    escaped = re.escape(value)
    # Render actual controls as explicit regex escapes, not raw file newlines.
    escaped = escaped.replace("\\\n", "\\x0a").replace("\\\r", "\\x0d").replace("\\\t", "\\x09")
    if len(escaped.encode("utf-8")) > MAX_LITERAL_BYTES:
        _fail("pattern_too_long", "转义后的正则超过当前规则格式上限")
    return escaped


def _predicate(node: tuple) -> tuple[str, str | None, str]:
    _, field, value = node
    if field in {"body", "web.body"}:
        return "bodyRegex", None, _literal(value)
    if field in {"title", "web.title"}:
        if any(character in value for character in "<>&"):
            _fail("unsupported_title_encoding", "不能无依据推断标题 HTML 实体、标记或解码规则")
        pattern = rf"(?i:<title\b[^>]*>)[^<]*{_literal(value)}[^<]*(?i:</title\s*>)"
        if len(pattern.encode("utf-8")) > MAX_LITERAL_BYTES:
            _fail("pattern_too_long", "标题正则超过当前规则格式上限")
        return "bodyRegex", None, pattern
    if field == "server":
        return "headerRegex", "server", _literal(value)
    if field == "header":
        if "\r" in value or "\n" in value:
            _fail("unsupported_header_shape", "不将多行原始响应头缩减为单一头部条件")
        name, separator, content = value.partition(":")
        if not separator or not re.fullmatch(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+", name):
            _fail("unsupported_header_shape", "通用 header 查询必须明确指定一个 HeaderName: value")
        name = name.lower()
        if re.search(r"authorization|cookie|token|secret|session|api[-_]?key", name) or name in {"www-authenticate", "authentication-info"}:
            _fail("unsupported_sensitive_header", "Cookie、认证或密钥头部不能自动迁移为公开识别指纹")
        # HeaderMap stores field values, without the serialized 'Name: ' prefix.
        # Anchoring retains its value-prefix requirement rather than accepting a
        # matching word anywhere in an unrelated portion of the value.
        content = content.lstrip(" \t")
        return "headerRegex", name, r"\A" + _literal(content)
    _fail("unsupported_field", f"字段 {field} 不能由当前识别模型无依据表达")


def _flat_predicates(query: str) -> tuple[str, list[tuple]]:
    expression = _simplify(_Parser(normalize_query(query)).parse())
    if expression[0] == "predicate":
        mode, predicates = "any", [expression]
    else:
        if any(child[0] != "predicate" for child in expression[1:]):
            _fail("mixed_boolean", "当前规则只支持平面 any/all，不能摊平混合 AND/OR")
        mode = "all" if expression[0] == "and" else "any"
        predicates = list(expression[1:])
    if len(predicates) > 128:
        _fail("too_many_signals", "原子条件超过当前规则格式上限")
    return mode, predicates


def query_predicates(query: str) -> list[dict[str, str]]:
    """Expose parsed literal evidence for caller quality/privacy gates.

    Values come from the strict parser, not reverse parsing generated regexes.
    The same complete supported boolean-shape checks as convert_query apply.
    convert_query remains responsible for rejecting unsupported HTTP fields.
    """
    _, predicates = _flat_predicates(query)
    return [{"field": predicate[1], "value": predicate[2]} for predicate in predicates]


def convert_query(query: str) -> dict[str, Any]:
    """Return matcher fields only; the caller assigns product IDs and provenance.

    No constraints are dropped. Pure nested AND/OR is flattened, exact duplicate
    subexpressions collapse, and any remaining mixed boolean tree is rejected.
    """
    mode, predicates = _flat_predicates(query)
    body_patterns: set[str] = set()
    header_patterns: dict[str, set[str]] = {}
    for predicate in predicates:
        kind, name, pattern = _predicate(predicate)
        if kind == "bodyRegex":
            body_patterns.add(pattern)
        else:
            header_patterns.setdefault(name, set()).add(pattern)
    result: dict[str, Any] = {"matchMode": mode}
    if body_patterns:
        result["bodyRegex"] = sorted(body_patterns)
    if header_patterns:
        headers: dict[str, str] = {}
        for name, patterns in sorted(header_patterns.items()):
            if len(patterns) == 1:
                headers[name] = next(iter(patterns))
            elif mode == "all":
                _fail("same_header_conjunction", f"同一响应头 {name} 的多个 AND 条件无法放入单值映射")
            else:
                pattern = "(?:" + "|".join(sorted(patterns)) + ")"
                if len(pattern.encode("utf-8")) > MAX_LITERAL_BYTES:
                    _fail("pattern_too_long", "合并后的响应头正则超过当前规则格式上限")
                headers[name] = pattern
        result["headerRegex"] = headers
    return result


def query_predicate_tree(query: str) -> dict[str, Any]:
    """Return the complete supported literal AST, without flattening mixed logic.

    Every predicate is validated with the same field/escape checks as the matcher
    conversion. The returned values are source data for privacy/specificity gates.
    """
    expression = _simplify(_Parser(normalize_query(query)).parse())
    count = 0

    def walk(node: tuple, depth: int) -> dict[str, Any]:
        nonlocal count
        count += 1
        if depth > 16 or count > 256:
            _fail('query_too_complex', '条件树超过 16 层或 256 个节点')
        if node[0] == 'predicate':
            _predicate(node)  # Validate the whole input; no unsupported clause drops.
            return {'type': 'literal', 'field': node[1], 'value': node[2]}
        return {'type': 'all' if node[0] == 'and' else 'any',
                'children': [walk(child, depth + 1) for child in node[1:]]}

    return walk(expression, 0)


def query_predicates_tree(query: str) -> list[dict[str, str]]:
    """All validated source literals, including leaves under mixed AND/OR."""
    result: list[dict[str, str]] = []

    def walk(node: dict[str, Any]) -> None:
        if node['type'] == 'literal':
            result.append({'field': node['field'], 'value': node['value']})
        else:
            for child in node['children']:
                walk(child)

    walk(query_predicate_tree(query))
    return result


def convert_query_tree(query: str) -> dict[str, Any]:
    """Preserve old Any serialization; use an explicit tree for AND/mixed logic.

    Existing single/flat OR fingerprints retain the exact old fields so their
    canonical signatures and IDs do not change. New tree leaves retain repeated
    same-header conjunctions instead of overwriting a dictionary entry.
    """
    source = query_predicate_tree(query)
    try:
        legacy = convert_query(query)
    except ConversionError as error:
        if error.code not in {'mixed_boolean', 'same_header_conjunction'}:
            raise
    else:
        if legacy['matchMode'] == 'any':
            return legacy

    def walk(node: dict[str, Any]) -> dict[str, Any]:
        if node['type'] != 'literal':
            return {'type': node['type'], 'children': [walk(child) for child in node['children']]}
        kind, name, pattern = _predicate(('predicate', node['field'], node['value']))
        if kind == 'bodyRegex':
            return {'type': 'body', 'pattern': pattern}
        return {'type': 'header', 'name': name, 'pattern': pattern}

    return {'condition': walk(source)}
