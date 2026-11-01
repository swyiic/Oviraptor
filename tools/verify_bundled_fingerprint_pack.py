#!/usr/bin/env python3
"""Read-only preflight for explicitly indexed, identification-only resource packs.

Uses the same documented canonical JSON representation as fingerprint_pack.rs for
rule checksums. This does not implement Rust regex syntax or product accuracy
checks; the production Rust validator remains authoritative for regex semantics.
No network, application startup, user data or database access is performed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import unicodedata
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_DIRECTORY = ROOT / 'src-tauri/resources/fingerprint-packs'
MAX_PACK_BYTES = 5 * 1024 * 1024
MAX_RULES = 5000
RUNTIME = 'condition-tree-v1'
SHA = re.compile(r'[0-9a-fA-F]{64}\Z')
ID = re.compile(r'[a-z0-9-]{1,128}\Z')
HEADER = re.compile(r"[!#$%&'*+.^_`|~0-9A-Za-z-]{1,128}\Z")


class VerificationError(ValueError):
    pass


def fail(message: str):
    raise VerificationError(message)


def object_fields(value, allowed, required, label):
    if not isinstance(value, dict):
        fail(f'{label}: expected object')
    if set(value) - set(allowed) or set(required) - set(value):
        fail(f'{label}: unknown or missing fields')


def text(value, maximum, label, required=True):
    if not isinstance(value, str) or len(value.encode('utf-8')) > maximum:
        fail(f'{label}: invalid string or byte length')
    if (required and not value.strip()) or any(unicodedata.category(c) in {'Cc', 'Cs'} for c in value):
        fail(f'{label}: empty text or control characters')
    return value


def integer(value, lower, upper, label):
    if type(value) is not int or not lower <= value <= upper:
        fail(f'{label}: invalid integer')
    return value


def checksum(value, label):
    if not isinstance(value, str) or not SHA.fullmatch(value):
        fail(f'{label}: invalid SHA-256')
    return value.lower()


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'))


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            fail('JSON contains duplicate object keys')
        result[key] = value
    return result


def bounded_json(path: Path, maximum: int):
    # O_NOFOLLOW protects the final component where the platform supports it.
    flags = os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0)
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, 'rb') as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > maximum:
            fail(f'{path.name}: not a bounded ordinary file')
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        fail(f'{path.name}: file grew beyond byte limit')
    try:
        value = json.loads(raw.decode('utf-8'), object_pairs_hook=unique_object)
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        fail(f'{path.name}: invalid UTF-8 JSON ({type(error).__name__})')
    return raw, value


def resource_path(directory: Path, relative: str):
    text(relative, 1024, 'manifest file')
    parts = PurePosixPath(relative).parts
    if not parts or relative.startswith('/') or '\\' in relative or ':' in relative or any(p in {'.', '..'} for p in parts):
        fail('manifest file: unsafe relative path')
    # PurePosixPath normalizes a single dot, so explicitly reject it in raw input.
    if any(p in {'.', '..', ''} for p in relative.split('/')):
        fail('manifest file: ambiguous path component')
    candidate = directory
    for part in parts:
        candidate = candidate / part
        if candidate.is_symlink():
            fail('manifest file: symbolic links are not allowed')
    if candidate.suffix.lower() != '.json' or not candidate.is_file():
        fail('manifest file: expected ordinary JSON file')
    if not candidate.resolve(strict=True).is_relative_to(directory):
        fail('manifest file escapes resource directory')
    return candidate


def pattern(value, label):
    # Do not use Python re.compile: this format deliberately uses Rust regexes.
    return text(value, 4096, label)


def array(value, maximum, label):
    if not isinstance(value, list) or len(value) > maximum:
        fail(f'{label}: invalid array or length')
    return value


def condition(value, depth=1, budget=None):
    if budget is None:
        budget = [0]
    budget[0] += 1
    if depth > 16 or budget[0] > 256:
        fail('condition exceeds depth or node limit')
    if not isinstance(value, dict):
        fail('condition must be object')
    kind = value.get('type')
    if kind in {'all', 'any'}:
        object_fields(value, {'type', 'children'}, {'type', 'children'}, 'condition group')
        children = array(value['children'], 256, 'condition children')
        if not children:
            fail('condition groups must not be empty')
        normalized = [condition(child, depth + 1, budget) for child in children]
        normalized.sort(key=compact)
        unique = {compact(child): child for child in normalized}
        return {'type': kind, 'children': list(unique.values())}
    if kind in {'body', 'cookie'}:
        object_fields(value, {'type', 'pattern'}, {'type', 'pattern'}, 'condition leaf')
        return {'type': kind, 'pattern': pattern(value['pattern'], 'condition pattern')}
    if kind == 'header':
        object_fields(value, {'type', 'name', 'pattern'}, {'type', 'name', 'pattern'}, 'header condition')
        name = text(value['name'], 128, 'header name')
        if not HEADER.fullmatch(name):
            fail('invalid header name')
        return {'type': kind, 'name': name.lower(), 'pattern': pattern(value['pattern'], 'header pattern')}
    fail('unknown condition type')


def path_signal(value):
    object_fields(value, {'path', 'status', 'bodyRegex'}, {'path', 'bodyRegex'}, 'path signal')
    path = text(value['path'], 1024, 'path')
    lowered = path.lower()
    if (not path.startswith('/') or path.startswith('//') or any(c.isspace() or c in '\\:?#' for c in path)
            or any(part in {'.', '..'} for part in path.split('/'))
            or any(token in lowered for token in ('%2e', '%2f', '%5c', '%0d', '%0a', '%00', '%3a', '%3f'))):
        fail('unsafe characteristic path')
    result = {'path': path}
    status = sorted(set(integer(v, 100, 599, 'HTTP status') for v in array(value.get('status', []), 500, 'HTTP statuses')))
    if status:
        result['status'] = status
    result['bodyRegex'] = pattern(value['bodyRegex'], 'path bodyRegex')
    return result


def fingerprint(value):
    allowed = {'id', 'name', 'headerRegex', 'setCookieRegex', 'bodyRegex', 'pathSignals', 'matchMode', 'condition'}
    object_fields(value, allowed, {'id', 'name'}, 'fingerprint')
    identifier = text(value['id'], 128, 'fingerprint ID')
    if not ID.fullmatch(identifier):
        fail('fingerprint ID must use lowercase ASCII letters, digits or hyphens')
    result = {'id': identifier, 'name': text(value['name'], 256, 'fingerprint name')}
    mode = value.get('matchMode', 'any')
    if mode != 'any':
        fail('flat matchMode is not supported; use condition tree')
    headers = value.get('headerRegex', {})
    if not isinstance(headers, dict):
        fail('headerRegex must be object')
    cookies = array(value.get('setCookieRegex', []), 128, 'cookie patterns')
    bodies = array(value.get('bodyRegex', []), 128, 'body patterns')
    paths = array(value.get('pathSignals', []), 32, 'path signals')
    signals = len(headers) + len(cookies) + len(bodies) + len(paths)
    if value.get('condition') is not None:
        if signals:
            fail('condition tree cannot be mixed with legacy signals')
        result['condition'] = condition(value['condition'])
        return result
    if not 1 <= signals <= 128:
        fail('fingerprint must contain 1..128 signals')
    canonical_headers = {}
    for name, value_pattern in headers.items():
        text(name, 128, 'header name')
        if not HEADER.fullmatch(name):
            fail('invalid header name')
        normalized = name.lower()
        value_pattern = pattern(value_pattern, 'header regex')
        if normalized in canonical_headers and canonical_headers[normalized] != value_pattern:
            fail('conflicting header names after case normalization')
        canonical_headers[normalized] = value_pattern
    if canonical_headers:
        result['headerRegex'] = dict(sorted(canonical_headers.items()))
    for field, values in [('setCookieRegex', cookies), ('bodyRegex', bodies)]:
        values = sorted(set(pattern(v, field) for v in values))
        if values:
            result[field] = values
    if paths:
        paths = [path_signal(v) for v in paths]
        paths.sort(key=lambda v: (v['path'], v.get('status', []), v['bodyRegex']))
        result['pathSignals'] = list({compact(v): v for v in paths}.values())
    return result


def validate_pack(value):
    allowed = {'schemaVersion', 'kind', 'booleanRuntime', 'manifest', 'fingerprints', 'sha256'}
    object_fields(value, allowed, {'schemaVersion', 'kind', 'manifest', 'fingerprints', 'sha256'}, 'pack')
    schema = integer(value['schemaVersion'], 1, 2, 'pack schemaVersion')
    if value['kind'] != 'oviraptor-fingerprints':
        fail('unknown fingerprint pack kind')
    if schema == 2 and value.get('booleanRuntime') != RUNTIME:
        fail('schema 2 requires condition-tree-v1 runtime')
    if schema == 1 and 'booleanRuntime' in value:
        fail('schema 1 cannot declare boolean runtime')
    metadata = value['manifest']
    object_fields(metadata, {'name', 'version', 'sources'}, {'name', 'version'}, 'pack manifest')
    text(metadata['name'], 256, 'pack name')
    text(metadata['version'], 64, 'pack version')
    for source in array(metadata.get('sources', []), 128, 'pack sources'):
        object_fields(source, {'name', 'repository', 'revision', 'license'}, set(), 'pack source')
        for key, maximum in [('name', 256), ('repository', 1024), ('revision', 128), ('license', 128)]:
            if key in source:
                text(source[key], maximum, 'source ' + key, required=False)
        if source.get('repository'):
            uri = urlsplit(source['repository'])
            if uri.scheme != 'https' or not uri.netloc or '@' in uri.netloc or uri.username or uri.password or uri.query or uri.fragment:
                fail('source repository must be credential-free HTTPS')
    rows = array(value['fingerprints'], MAX_RULES, 'fingerprints')
    if not rows:
        fail('bundled fingerprint pack must not be empty')
    rules = [fingerprint(row) for row in rows]
    identifiers = [rule['id'] for rule in rules]
    if len(set(identifiers)) != len(identifiers):
        fail('duplicate fingerprint ID inside pack')
    if schema == 1 and any('condition' in rule for rule in rules):
        fail('condition tree requires schemaVersion 2')
    rules.sort(key=lambda rule: rule['id'])
    computed = hashlib.sha256(compact(rules).encode('utf-8')).hexdigest()
    if computed != checksum(value['sha256'], 'rule checksum'):
        fail('canonical rule-content SHA-256 mismatch')
    return rules, computed


def verify(directory: Path):
    directory = Path(directory)
    if directory.is_symlink() or not directory.is_dir():
        fail('resource directory must be a real directory')
    directory = directory.resolve(strict=True)
    manifest_path = directory / 'manifest.json'
    if manifest_path.is_symlink():
        fail('manifest may not be a symbolic link')
    _, manifest = bounded_json(manifest_path, 1024 * 1024)
    object_fields(manifest, {'schemaVersion', 'packs'}, {'schemaVersion', 'packs'}, 'resource manifest')
    if type(manifest['schemaVersion']) is not int or manifest['schemaVersion'] != 1:
        fail('unsupported resource manifest version')
    packs = array(manifest['packs'], 64, 'indexed packs')
    if not packs:
        fail('resource manifest has no packs')
    identifiers = set()
    filenames = set()
    checked = []
    for entry in packs:
        object_fields(entry, {'file', 'sha256', 'count', 'ruleSha256'}, {'file', 'sha256', 'count', 'ruleSha256'}, 'pack entry')
        relative = text(entry['file'], 1024, 'manifest file')
        if relative in filenames:
            fail('duplicate indexed file')
        filenames.add(relative)
        path = resource_path(directory, relative)
        raw, value = bounded_json(path, MAX_PACK_BYTES)
        file_digest = hashlib.sha256(raw).hexdigest()
        if file_digest != checksum(entry['sha256'], 'whole-file checksum'):
            fail(f'{path.name}: whole-file SHA-256 mismatch')
        rules, content_digest = validate_pack(value)
        if len(rules) != integer(entry['count'], 1, MAX_RULES, 'pack count'):
            fail(f'{path.name}: fingerprint count mismatch')
        if content_digest != checksum(entry['ruleSha256'], 'manifest rule checksum'):
            fail(f'{path.name}: indexed rule-content checksum mismatch')
        for rule in rules:
            if rule['id'] in identifiers:
                fail('duplicate fingerprint ID across indexed packs')
            identifiers.add(rule['id'])
        if len(identifiers) > MAX_RULES:
            fail('aggregate bundled rule limit exceeded')
        checked.append({'file': entry['file'], 'count': len(rules), 'sha256': file_digest, 'ruleSha256': content_digest})
    return {'valid': True, 'packs': checked, 'uniqueFingerprints': len(identifiers), 'regexSemantics': 'validated by production Rust separately', 'networkRequests': 0, 'databaseAccess': False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, default=DEFAULT_DIRECTORY)
    arguments = parser.parse_args(argv)
    try:
        result = verify(arguments.directory)
    except (VerificationError, OSError, UnicodeError, TypeError, ValueError, RecursionError) as error:
        print(json.dumps({'valid': False, 'error': str(error)}, ensure_ascii=False), file=sys.stderr)
        return 1
    print(json.dumps(result, ensure_ascii=False, separators=(',', ':')))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
