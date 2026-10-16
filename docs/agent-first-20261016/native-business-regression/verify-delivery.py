#!/usr/bin/env python3
"""Verify only this evidence delivery; never run project code/tests."""
import hashlib,json
from pathlib import Path
root=Path(__file__).resolve().parent
idx=json.loads((root/'delivery-index.json').read_text())
errors=[]
for ref in idx['files']:
 p=root/ref['path']
 if not p.is_relative_to(root) or p.is_symlink() or not p.is_file():
  errors.append(ref['path']+': absent/nonregular'); continue
 h=hashlib.sha256(p.read_bytes()).hexdigest()
 if h!=ref['sha256'] or p.stat().st_size!=ref['bytes']:
  errors.append(ref['path']+': size/hash mismatch')
actual={str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()}
expected={r['path'] for r in idx['files']}|{'delivery-index.json'}
if actual!=expected: errors.append('file path set mismatch')
for ref in idx['copiedOriginals']:
 p=root/ref['packagePath']
 if hashlib.sha256(p.read_bytes()).hexdigest()!=ref['sha256']:
  errors.append(ref['packagePath']+': original/copy mismatch')
print(json.dumps({'accepted':not errors,'filesChecked':len(idx['files']),
 'byteExactOriginalCopiesChecked':len(idx['copiedOriginals']),
 'cargoOrProjectTestInvoked':False,'errors':errors},indent=2))
raise SystemExit(1 if errors else 0)
