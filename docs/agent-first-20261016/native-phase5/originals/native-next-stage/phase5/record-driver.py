#!/usr/bin/env python3
"""Independently record an actual helper process return; never prefill success."""
import datetime,hashlib,json,pathlib,re,subprocess,sys,time
OUT=pathlib.Path(__file__).resolve().parent
prefix,*argv=sys.argv[1:]
if not argv or not re.fullmatch('[A-Za-z0-9][A-Za-z0-9._-]*',prefix):raise SystemExit('Fresh label/argv required')
receipt=OUT/(prefix+'-actual-outer-receipt.json')
if receipt.exists():raise SystemExit('Receipt already exists')
started=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic();result=subprocess.run(argv)
refs=[]
for p in sorted(OUT.glob(prefix+'*')):
 if p.is_file() and p!=receipt:
  with p.open('rb') as f:h=hashlib.file_digest(f,'sha256').hexdigest()
  refs.append({'path':str(p),'bytes':p.stat().st_size,'sha256':h})
with receipt.open('x') as f:json.dump({'argv':argv,'startedAtUtc':started,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'durationSeconds':time.monotonic()-tick,'actualProcessExitCode':result.returncode,'observedAfterActualProcessReturn':True,'resultPrefixReferences':refs},f,indent=2);f.write('\n')
print(json.dumps({'receipt':str(receipt),'actualProcessExitCode':result.returncode}));raise SystemExit(result.returncode)
