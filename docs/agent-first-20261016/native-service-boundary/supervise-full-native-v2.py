#!/usr/bin/env python3
import datetime,hashlib,json,pathlib,subprocess,sys,time
out=pathlib.Path(__file__).resolve().parent
receipt=out/'service-native-full-02-independent-outer-receipt.json'
if receipt.exists():raise RuntimeError('Fresh outer receipt required')
argv=[sys.executable,str(out/'run-full-native-v2.py'),'--execute']
started=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic()
r=subprocess.run(argv)
finished=datetime.datetime.now(datetime.timezone.utc).isoformat()
def sha_if(p):
 if not p.exists():return None
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
v={'startedAtUtc':started,'finishedAtUtc':finished,'durationSeconds':round(time.monotonic()-tick,3),'argv':argv,'actualProcessExitCode':r.returncode,'nativeCommittedBaseline':'3d2eca1cc2aa3f0c85a51bdf8fe125e0a7659d15','aggregateSha256':sha_if(out/'service-native-full-02-aggregate.json'),'actualLogSha256':sha_if(out/'service-native-full-02-command.log'),'supervisorSha256':sha_if(pathlib.Path(__file__))}
with receipt.open('x') as f:json.dump(v,f,indent=2);f.write('\n')
print(json.dumps({'independentOuterReceipt':str(receipt),'actualProcessExitCode':r.returncode,'durationSeconds':v['durationSeconds']}))
raise SystemExit(r.returncode)
