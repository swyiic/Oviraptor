#!/usr/bin/env python3
import datetime,json,pathlib,subprocess,sys,time
out=pathlib.Path(__file__).resolve().parent;root=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
argv=[sys.executable,str(out/'run-isolated.py'),'phase2-dev-bins-build-01','cargo','build','--manifest-path','src-tauri/Cargo.toml','--locked','--offline','--bin','oviraptor-scan-service','--bin','oviraptor-assets-service','--jobs','2','--message-format=json']
start=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic();result=subprocess.run(argv,cwd=root)
with (out/'phase2-dev-bins-build-01-independent-receipt.json').open('x') as f:json.dump({'argv':argv,'startedAtUtc':start,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'durationSeconds':time.monotonic()-tick,'actualProcessExitCode':result.returncode},f,indent=2);f.write('\n')
if result.returncode:sys.exit(result.returncode)
argv=[sys.executable,str(out/'seal-dev-binaries.py'),'--build-prefix','phase2-dev-bins-build-01','--result-name','phase2-dev-binaries-01.json'];result=subprocess.run(argv,cwd=root)
with (out/'phase2-dev-binaries-01-independent-receipt.json').open('x') as f:json.dump({'argv':argv,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'actualProcessExitCode':result.returncode},f,indent=2);f.write('\n')
sys.exit(result.returncode)
