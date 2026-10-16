#!/usr/bin/env python3
import datetime,hashlib,importlib.util,json,pathlib,re,subprocess,sys,time
out=pathlib.Path(__file__).resolve().parent;root=pathlib.Path('/workspace/Oviraptor-agent-first-20261016');dest=out/'paid-neighborhood-repeat-10';dest.mkdir()
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(p,v):
 with p.open('x') as f:json.dump(v,f,indent=2);f.write('\n')
spec=importlib.util.spec_from_file_location('record',out/'run-isolated.py');record=importlib.util.module_from_spec(spec);spec.loader.exec_module(record)
a=json.loads((out/'service-library-artifact.json').read_text());binary=out/'retained-113a-full-failure/oviraptor_lib-75f6df7542c7ea38';baseline=json.loads((out/'service-focused-01-after.json').read_text())['nativeInputs'];expected_sha=a['binarySha256'];plan=json.loads((out/'paid-neighborhood-selection-01.json').read_text());names=plan['names']
if binary.is_symlink() or sha(binary)!=expected_sha or len(names)!=15 or len(set(names))!=15:raise RuntimeError('Immutable binary or actual15 selection differs')
pre=record.snapshot()
if pre['nativeInputs']!=baseline:raise RuntimeError('Different native source')
save(dest/'before.json',pre);save(dest/'plan.json',{'roundsRequested':10,'uniqueCaseCount':15,'names':names,'threadsEachRound':8,'binary':str(binary),'binarySha256':expected_sha,'parameters':'All original test assertions/TTL unchanged; original8threadfull failure remains independently authoritative.'})
results=[];started=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic()
for number in range(1,11):
 label=f'service-paid-repeat-{number:02d}';command=[sys.executable,str(out/'run-isolated.py'),label,sys.executable,'tools/run_with_subreaper.py',str(binary),'--exact',*names,'--test-threads=8','--nocapture'];begin=datetime.datetime.now(datetime.timezone.utc).isoformat();rt=time.monotonic()
 with (dest/f'round-{number:02d}-outer.log').open('x') as stream:r=subprocess.run(command,cwd=root,stdout=stream,stderr=subprocess.STDOUT)
 finish=datetime.datetime.now(datetime.timezone.utc).isoformat();receipt=json.loads((out/(label+'-result.json')).read_text());text=(out/(label+'.log')).read_text(errors='replace');cases=re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?:[ \t].*)?$',text,re.M);summary=re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in ([0-9.]+)s$',text,re.M);post=record.snapshot()
 checks={'actualOuterZero':r.returncode==0,'nativeGuardAccepted':receipt['acceptedForNativeSource'],'native1774Unchanged':post['nativeInputs']==baseline,'immutableBinaryUnchanged':sha(binary)==expected_sha,'exact15CasesEachOnce':len(cases)==15 and {n for n,_ in cases}==set(names),'allPassed':all(s=='ok' for _,s in cases),'preciseCompleteSummary':len(summary)==1 and list(summary[0][:6])==['ok','15','0','0','0','3826']}
 row={'round':number,'accepted':all(checks.values()),'checks':checks,'actualProcessExitCode':r.returncode,'actualTestCommandExitCode':receipt['actualCommandExitCode'],'startedAtUtc':begin,'finishedAtUtc':finish,'durationSeconds':round(time.monotonic()-rt,3),'argv':command,'cases':cases,'summary':summary,'rawLog':str(out/(label+'.log')),'rawLogSha256':sha(out/(label+'.log')),'resultSha256':sha(out/(label+'-result.json')),'binarySha256After':sha(binary)}
 save(dest/f'round-{number:02d}-independent-receipt.json',row);results.append(row);print(json.dumps({k:row[k] for k in ['round','accepted','actualProcessExitCode','durationSeconds','summary']}),flush=True)
 if not row['accepted']:break
post=record.snapshot();save(dest/'after.json',post)
v={'accepted':len(results)==10 and all(row['accepted'] for row in results),'startedAtUtc':started,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'durationSeconds':round(time.monotonic()-tick,3),'uniqueCaseCount':15,'roundsRequested':10,'roundsCompleted':len(results),'totalCaseExecutions':sum(len(row['cases']) for row in results),'caseCountMeaning':'Repeated runs of15 unique cases; never150 distinct cases.','rounds':results,'native1774EqualsFocusedAfter':post['nativeInputs']==baseline,'binarySha256':sha(binary),'originalFullStatus':'Original3841/8threads full had3833passed1failed7ignored/actual101; repeat passing does not remove that failure or establish a cause.'}
save(dest/'repeat-result.json',v);print(json.dumps({k:v[k] for k in ['accepted','roundsCompleted','uniqueCaseCount','totalCaseExecutions','durationSeconds']}),flush=True)
raise SystemExit(0 if v['accepted'] else (results[-1]['actualProcessExitCode'] or 1))
