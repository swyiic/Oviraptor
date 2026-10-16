from pathlib import Path
import datetime, hashlib, json, os, re, subprocess, time
root=Path('/workspace/Oviraptor-agent-first-20261016')
out=Path(__file__).resolve().parent
files=set(root.joinpath('src').rglob('*.ts'))
files.update(root.joinpath('src-tauri/resources/workers').rglob('*.cjs'))
files.update(root/p for p in ['tools/api_owner_harness.cjs','tools/test_api_owner_contracts.cjs','tools/test_native_runtime.cjs','src-tauri/resources/workers/tests/fixtures/runtime_probe.html','src-tauri/resources/config/browser-locations.json','package-lock.json'])
def snapshot():
    return {str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files) if p.is_file()}
before=snapshot();(out/'targeted-source-before.json').write_text(json.dumps(before,indent=2)+'\n')
head_before=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
status_before=subprocess.check_output(['git','status','--porcelain'],cwd=root,text=True).splitlines()
command=['node','--test','--test-concurrency=1','--test-reporter=tap','tools/test_api_owner_contracts.cjs','tools/test_native_runtime.cjs']
env={**os.environ,'CHROME_BINARY':'/usr/bin/chromium','OVIRAPTOR_TEST_NO_SANDBOX':'1'}
started=datetime.datetime.now(datetime.timezone.utc).isoformat();start=time.monotonic()
with (out/'targeted.log').open('w') as log:
    completed=subprocess.run(command,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
finished=datetime.datetime.now(datetime.timezone.utc).isoformat();duration=time.monotonic()-start
after=snapshot();(out/'targeted-source-after.json').write_text(json.dumps(after,indent=2)+'\n')
raw=(out/'targeted.log').read_text();summary={m.group(1):int(m.group(2)) for m in re.finditer(r'^# (tests|suites|pass|fail|cancelled|skipped|todo) (\d+)$',raw,re.M)}
cases=[{'status':m.group(1),'ordinal':int(m.group(2)),'name':m.group(3)} for m in re.finditer(r'^(ok|not ok) (\d+) - (.+)$',raw,re.M)]
changes={'added':sorted(set(after)-set(before)),'removed':sorted(set(before)-set(after)),'changed':sorted(k for k in set(before)&set(after) if before[k]!=after[k])}
original=json.loads((out/'before-and-cause.json').read_text())
preserved={p:after[p]==value for p,value in original['files'].items() if p!='tools/api_owner_harness.cjs'}
result={'command':command,'environment':{'CHROME_BINARY':env['CHROME_BINARY'],'OVIRAPTOR_TEST_NO_SANDBOX':env['OVIRAPTOR_TEST_NO_SANDBOX']},'startedAtUtc':started,'finishedAtUtc':finished,'actualExit':completed.returncode,'elapsedSeconds':round(duration,3),'summary':summary,'caseResults':cases,'sourceHeadBefore':head_before,'sourceHeadAfter':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'gitStatusBefore':status_before,'gitStatusAfter':subprocess.check_output(['git','status','--porcelain'],cwd=root,text=True).splitlines(),'inputCount':len(before),'sourceChanges':changes,'selectedInputsStable':not any(changes.values()),'originalBodiesAndProductFilesPreserved':preserved,'testScope':'Actual legacy API methods with only Tauri IPC substituted at exact Assets uiPorts seam, and synthetic loopback CDP with explicit test-only Linux no-sandbox opt-in. This is not product browser sandbox acceptance.','noProductOrRustChangesByThisTask':True,'noCommitOrStageByThisTask':True,'rawLog':str(out/'targeted.log'),'rawLogSha256':hashlib.sha256((out/'targeted.log').read_bytes()).hexdigest()}
result['targetedAccepted']=completed.returncode==0 and summary.get('tests')==6 and summary.get('pass')==6 and summary.get('fail')==0 and summary.get('skipped')==0 and result['selectedInputsStable'] and all(preserved.values())
(out/'targeted-result.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:result[k] for k in ['actualExit','elapsedSeconds','summary','inputCount','selectedInputsStable','targetedAccepted']},indent=2),flush=True)
raise SystemExit(completed.returncode)
