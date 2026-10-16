#!/usr/bin/env python3
import argparse,datetime,hashlib,json,pathlib,subprocess,sys,time
out=pathlib.Path(__file__).resolve().parent
p=argparse.ArgumentParser();p.add_argument('--execute',action='store_true');p.add_argument('--label',required=True);args=p.parse_args();assert args.execute
receipt=out/(args.label+'-independent-supervisor-receipt.json');assert not receipt.exists()
argv=[sys.executable,str(out/'run-native-full.py'),'--execute','--label',args.label];start=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic();result=subprocess.run(argv,cwd='/workspace/Oviraptor-agent-first-20261016');finished=datetime.datetime.now(datetime.timezone.utc).isoformat()
def digest(path):
 if not path.is_file():return None
 with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
data={'argv':argv,'startedAtUtc':start,'finishedAtUtc':finished,'durationSeconds':time.monotonic()-tick,'actualProcessExitCode':result.returncode,'aggregateSha256':digest(out/(args.label+'-aggregate.json')),'logSha256':digest(out/(args.label+'-command.log')),'scope':'Independent actual subprocess completion receipt, written only after the native full runner returned. No prefilled success.'}
with receipt.open('x') as f:json.dump(data,f,indent=2);f.write('\n')
print(json.dumps(data));sys.exit(result.returncode)
