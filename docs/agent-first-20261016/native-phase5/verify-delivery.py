#!/usr/bin/env python3
import hashlib,json
from pathlib import Path
r=Path(__file__).resolve().parent;x=json.loads((r/'delivery-index.json').read_text());errors=[]
for f in x['files']:
 p=r/f['path']
 if not p.is_relative_to(r) or not p.is_file() or p.is_symlink():errors.append(f['path']+': absent/nonregular');continue
 if p.stat().st_size!=f['bytes'] or hashlib.sha256(p.read_bytes()).hexdigest()!=f['sha256']:errors.append(f['path']+': size/hash mismatch')
actual={str(p.relative_to(r)) for p in r.rglob('*') if p.is_file()};expected={f['path'] for f in x['files']}|{'delivery-index.json'}
if actual!=expected:errors.append('file path set mismatch')
for f in x['byteExactOriginals']:
 if hashlib.sha256((r/f['packagePath']).read_bytes()).hexdigest()!=f['sha256']:errors.append(f['packagePath']+': original mismatch')
print(json.dumps({'accepted':not errors,'filesChecked':len(x['files']),'originalCopiesChecked':len(x['byteExactOriginals']),'errors':errors,'cargoOrNativeOrProjectTestsInvoked':False},indent=2));raise SystemExit(1 if errors else 0)
