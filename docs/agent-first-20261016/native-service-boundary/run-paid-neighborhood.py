#!/usr/bin/env python3
import datetime,hashlib,json,pathlib,re,subprocess,sys,time
out=pathlib.Path(__file__).resolve().parent
root=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(n,v):
 with (out/n).open('x') as f:json.dump(v,f,indent=2);f.write('\n')
a=json.loads((out/'service-library-artifact.json').read_text());binary=pathlib.Path(a['cargoArtifact']['executable'])
if sha(binary)!=a['binarySha256']:raise RuntimeError('Different binary')
names=[r[:-6] for r in (out/'service-list-01.log').read_text().splitlines() if r.endswith(': test')]
selected=sorted(n for n in names if n.startswith('scanning::agent_tests::lease_paid_'))
if len(selected)!=len(set(selected)) or len(selected)!=15:raise RuntimeError('Actual expected fifteen named cases required')
command=[sys.executable,'tools/run_with_subreaper.py',str(binary),'--exact',*selected,'--test-threads=8','--nocapture']
save('paid-neighborhood-selection-01.json',{'names':selected,'selectedCount':len(selected),'actualListedCount':len(names),'command':command,'binarySha256':a['binarySha256'],'sourceListSha256':sha(out/'service-list-01.log')})
argv=[sys.executable,str(out/'run-isolated.py'),'service-paid-neighborhood-01',*command];started=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic();r=subprocess.run(argv,cwd=root);finished=datetime.datetime.now(datetime.timezone.utc).isoformat()
v={'startedAtUtc':started,'finishedAtUtc':finished,'durationSeconds':round(time.monotonic()-tick,3),'actualProcessExitCode':r.returncode,'argv':argv,'binarySha256Before':a['binarySha256'],'binarySha256After':sha(binary),'runnerSha256':sha(pathlib.Path(__file__)),'logSha256':sha(out/'service-paid-neighborhood-01.log'),'resultSha256':sha(out/'service-paid-neighborhood-01-result.json')}
save('paid-neighborhood-independent-outer-01.json',v)
text=(out/'service-paid-neighborhood-01.log').read_text(errors='replace');cases=re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?:[ \t].*)?$',text,re.M);summary=re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in ([0-9.]+)s$',text,re.M)
save('paid-neighborhood-case-result-01.json',{'cases':cases,'caseSetEachOnce':len(cases)==15 and {n for n,_ in cases}==set(selected),'actualOuterExitCode':r.returncode,'summary':summary,'selected':selected})
print(json.dumps({'outerExitCode':r.returncode,'cases':len(cases),'failed':[n for n,s in cases if s=='FAILED'],'summary':summary}))
raise SystemExit(r.returncode)
