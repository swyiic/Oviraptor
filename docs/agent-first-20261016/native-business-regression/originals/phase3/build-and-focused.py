#!/usr/bin/env python3
import datetime,json,pathlib,subprocess,sys,time
out=pathlib.Path(__file__).resolve().parent;root=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
check=json.loads((out/'phase3-all-targets-03-result.json').read_text());assert check['acceptedForNativeSource']
argv=[sys.executable,str(out/'run-isolated.py'),'phase3-main-lib-build-01','cargo','test','--manifest-path','src-tauri/Cargo.toml','--locked','--offline','--lib','--no-run','--message-format=json','--jobs','2'];start=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic();result=subprocess.run(argv,cwd=root)
with (out/'phase3-main-lib-build-01-independent-receipt.json').open('x') as f:json.dump({'argv':argv,'startedAtUtc':start,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'durationSeconds':time.monotonic()-tick,'actualProcessExitCode':result.returncode},f,indent=2);f.write('\n')
if result.returncode:sys.exit(result.returncode)
argv=[sys.executable,str(out/'run-main-focused.py'),'--build-prefix','phase3-main-lib-build-01','--result-prefix','phase3-focused-01'];start=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic();result=subprocess.run(argv,cwd=root)
with (out/'phase3-focused-01-independent-receipt.json').open('x') as f:json.dump({'argv':argv,'startedAtUtc':start,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'durationSeconds':time.monotonic()-tick,'actualProcessExitCode':result.returncode},f,indent=2);f.write('\n')
sys.exit(result.returncode)
