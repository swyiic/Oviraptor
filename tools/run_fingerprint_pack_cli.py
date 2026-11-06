#!/usr/bin/env python3
"""Compile/run the real production fingerprint-pack module offline.

No dependency download, application startup or database access. Only the explicit
'import DATA_DIR PACK EXPECTED_REVISION' command writes a user fingerprint file.
Commands and strict input/output rules are documented by the Rust adapter usage.
If no cached dependencies exist, build the project offline first.
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import verify_bundled_fingerprint_pack as resource_verifier

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tools/fingerprint_pack_cli.rs'
MODULE = ROOT / 'src-tauri/src/scanning/fingerprint_pack.rs'
BUILTINS = ROOT / 'src-tauri/resources/rules/fingerprints.json'
CURRENT = ROOT / 'src-tauri/src/scanning/exposure_probe.rs'
CURRENT_CHECKS = ROOT / 'tools/fingerprint_current_checks.rs'
LOGIC = ROOT / 'src-tauri/src/scanning/fingerprint_logic.rs'
RESOURCES = ROOT / 'src-tauri/resources/fingerprint-packs'
CRATES = ('tauri', 'serde', 'serde_json', 'sha2', 'regex', 'regex_syntax', 'reqwest', 'uuid')


def current_snapshot() -> str:
    source = CURRENT.read_text()
    def function(name: str) -> str:
        start = source.index('fn '+name+'(')
        # Selected functions use a column-zero closing brace only at their end.
        end = source.index('\n}', start)+2
        return source[start:end]+'\n'
    start = source.index('#[derive(Debug, Deserialize)]', source.index('// ---------------------------------------------------------------- fingerprints'))
    end = source.index('#[derive(Debug, Clone)]\npub(crate) struct Identified', start)
    model = source[start:end]
    start = source.index('struct Probe {')
    end = source.index('\n}',start)+2
    probe = source[start:end]
    boolean_start = source.rfind('#[derive',0,source.index('struct BooleanCatalog'))
    boolean_end = source.index('\n}',source.index('struct BooleanCatalog'))+2
    model += '\n'+source[boolean_start:boolean_end]+'\n'
    functions = '\n'.join(function(name) for name in [
        'starter_fingerprint_catalog','fingerprint_catalog_checked','fingerprint_catalog','user_rules_path','validate_fingerprint','parse_user_catalog',
        'read_user_fingerprints','boolean_rules_path','validate_input_match_mode','parse_boolean_catalog',
        'read_boolean_fingerprints','combined_user_rules','atomic_rules_write','write_user_fingerprints','write_boolean_fingerprints','merge_import','all_fingerprints','build_client','fetch','regex_matches','fingerprint_matches'])
    functions = functions.replace('"../../resources/rules/fingerprints.json"', json.dumps(str(BUILTINS)))
    header = ('use serde::Deserialize;\nuse std::path::Path;\nuse std::time::Duration;\nuse super::fingerprint_logic::{self,Condition,BOOLEAN_RUNTIME};\n'
              'const MAX_BODY:usize=65536;\n'
              f'const CURRENT_SOURCE_SHA256:&str={json.dumps(hashlib.sha256(CURRENT.read_bytes()).hexdigest())};\n'
              f'const CURRENT_SOURCE_PATH:&str={json.dumps(str(CURRENT))};\n')
    return header+model+'\n'+probe+'\n'+functions+'\n'+CURRENT_CHECKS.read_text()


def build() -> Path:
    version = subprocess.run(['rustc', '--version'], check=True, capture_output=True).stdout
    directories = [ROOT / 'src-tauri/target/debug/deps', ROOT / 'src-tauri/target/release/deps',
                   ROOT / 'target/debug/deps', ROOT / 'target/release/deps']
    dependencies = None
    for directory in directories:
        found = {name: sorted(directory.glob(f'lib{name}-*.rlib'), key=lambda p: p.stat().st_mtime_ns, reverse=True)
                 for name in CRATES}
        if all(found.values()):
            dependencies = {name: paths[0] for name, paths in found.items()}
            break
    if dependencies is None:
        raise RuntimeError('Cached Rust .rlib dependencies unavailable; run cargo build --offline in src-tauri first')
    verified = resource_verifier.verify(RESOURCES)
    resource_paths = [RESOURCES / item['file'] for item in verified['packs']]
    if any(path.parent.resolve() != RESOURCES.resolve() or not path.name.endswith('.fingerprints.json') for path in resource_paths):
        raise ValueError('Indexed packs must be flat *.fingerprints.json files')
    orphaned = {path.name for path in RESOURCES.glob('*.fingerprints.json')} - {path.name for path in resource_paths}
    if orphaned: raise ValueError('Unindexed fingerprint resources: '+', '.join(sorted(orphaned)))
    digest = hashlib.sha256(version)
    for source in (Path(__file__), SOURCE, MODULE, BUILTINS, CURRENT, CURRENT_CHECKS, LOGIC,
                   Path(resource_verifier.__file__), RESOURCES / 'manifest.json', *resource_paths):
        digest.update(source.read_bytes())
    for name, path in dependencies.items():
        digest.update(f'{name}:{path}:{path.stat().st_size}:{path.stat().st_mtime_ns}'.encode())
    cache_root = Path(tempfile.gettempdir()) / 'oviraptor-fingerprint-cli'
    if cache_root.is_symlink():
        raise RuntimeError('Refusing a symlinked compiler cache')
    cache_root.mkdir(mode=0o700, exist_ok=True)
    if hasattr(os, 'getuid') and cache_root.stat().st_uid != os.getuid():
        raise RuntimeError('Compiler cache belongs to another user')
    cache = cache_root / digest.hexdigest()[:24]
    if cache.is_symlink():
        raise RuntimeError('Refusing a symlinked build directory')
    binary = cache / ('fingerprint-pack.exe' if os.name == 'nt' else 'fingerprint-pack')
    if binary.is_symlink():
        raise RuntimeError('Refusing a symlinked executable')
    if binary.is_file():
        return binary
    cache.mkdir(mode=0o700, exist_ok=True)
    snapshot = cache / 'current_snapshot.rs'
    snapshot.write_text(current_snapshot())
    lines=['pub(crate) const BUNDLED_FINGERPRINT_PACKS: &[(&str, &str, &str, usize, &str)] = &[']
    for item, path in zip(verified['packs'], resource_paths):
        quote=lambda value:json.dumps(value,ensure_ascii=False)
        lines.append(f'({quote(item["file"])}, include_str!({quote(str(path.resolve()))}), {quote(item["sha256"])}, {item["count"]}, {quote(item["ruleSha256"])}),')
    lines.append('];')
    (cache / 'fingerprint_pack_resources.rs').write_text('\n'.join(lines)+'\n')
    command = ['rustc', '--edition=2021', '--crate-name', 'fingerprint_pack_cli', str(SOURCE), '-o', str(binary),
               '-L', f'dependency={directory}']
    for name, path in dependencies.items():
        command.extend(['--extern', f'{name}={path}'])
    (cache / 'compile-command.json').write_text(json.dumps(command, indent=2)+'\n')
    environment = dict(os.environ, CARGO_PKG_NAME='oviraptor', CARGO_MANIFEST_DIR=str(ROOT / 'src-tauri'), OVI_CURRENT_SNAPSHOT=str(snapshot), OUT_DIR=str(cache))
    result = subprocess.run(command, cwd=ROOT, env=environment, capture_output=True, text=True)
    (cache / 'compile.log').write_text(result.stdout+result.stderr)
    if result.returncode:
        raise RuntimeError(f'Rust adapter compilation failed; see {cache / "compile.log"}\n{result.stderr[-4000:]}')
    return binary


def main() -> int:
    try:
        binary = build()
        return subprocess.run([str(binary), *sys.argv[1:]], cwd=Path.cwd()).returncode
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        print(json.dumps({'error': str(error)}), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
