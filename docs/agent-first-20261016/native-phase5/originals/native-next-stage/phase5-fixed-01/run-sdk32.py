#!/usr/bin/env python3
import argparse,datetime,hashlib,json,os,pathlib,re,runpy,subprocess,sys,time
OUT=pathlib.Path(__file__).resolve().parent;ROOT=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
def sha(p):
 with pathlib.Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(name,x):
 with (OUT/name).open('x') as f:json.dump(x,f,indent=2);f.write('\n')
def main():
 p=argparse.ArgumentParser();p.add_argument('--label',required=True);a=p.parse_args();prefix=a.label;assert re.fullmatch('[A-Za-z0-9][A-Za-z0-9._-]*',prefix) and not list(OUT.glob(prefix+'*'))
 policy=json.loads((OUT/'declared-sdk32-policy.json').read_text());expected=policy['expectedCases'];snapshot=runpy.run_path(str(OUT/'run-isolated.py'))['snapshot'];before=snapshot();assert {p:h for p,h in before['nativeInputs'].items() if p.startswith('packages/native-app-sdk/')}==policy['sdkInputs']
 env=os.environ.copy();env['CARGO_TARGET_DIR']='/workspace/native-app-sdk-target'
 argv=[sys.executable,str(OUT/'run-isolated.py'),prefix,'cargo','test','--manifest-path','packages/native-app-sdk/Cargo.toml','--locked','--offline','--lib','--jobs','2','--message-format=json','--','--test-threads=2']
 started=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic();r=subprocess.run(argv,cwd=ROOT,env=env)
 save(prefix+'-independent-receipt.json',{'argv':argv,'environmentOverride':{'CARGO_TARGET_DIR':env['CARGO_TARGET_DIR']},'startedAtUtc':started,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'durationSeconds':time.monotonic()-tick,'actualProcessExitCode':r.returncode})
 result=json.loads((OUT/(prefix+'-result.json')).read_text());text=(OUT/(prefix+'.log')).read_text();rows=re.findall(r'^test (.+) \.\.\. (ok|FAILED|ignored)$',text,re.M);summaries=re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in ([0-9.]+)s',text)
 artifacts=[]
 for line in text.splitlines():
  try:item=json.loads(line)
  except ValueError:continue
  if item.get('reason')=='compiler-artifact' and item.get('target',{}).get('name')=='native_app_sdk' and item.get('executable'):artifacts.append(item)
 artifact_ok=False;binary=None
 if len(artifacts)==1:
  art=artifacts[0];pr=art['profile'];b=pathlib.Path(art['executable']);artifact_ok=art['features']==[] and art['target']['src_path']==str(ROOT/'packages/native-app-sdk/src/lib.rs') and pr['test'] and pr['opt_level']=='1' and pr['debug_assertions'] and pr['overflow_checks'] and b.is_file() and not b.is_symlink() and b.resolve().is_relative_to('/workspace/native-app-sdk-target/debug/deps')
  if artifact_ok:binary={'artifact':art,'path':str(b),'sha256':sha(b),'bytes':b.stat().st_size}
 checks={'realProcessExitZero':r.returncode==0,'nativeAndProtectedGuardsAccepted':result['acceptedForNativeSource'],'uniqueExpected32Rows':len(rows)==32==len({n for n,s in rows}) and {n for n,s in rows}==set(expected),'allRowsPassed':all(s=='ok' for n,s in rows),'original31NamesPreserved':set(policy['prior31CaseNames']).issubset({n for n,s in rows}),'exactCompleteSummary':len(summaries)==1 and summaries[0][0]=='ok' and tuple(map(int,summaries[0][1:6]))==(32,0,0,0,0),'realUniqueExpectedArtifact':artifact_ok}
 agg={'accepted':all(checks.values()),'scope':'Standalone SDK32 unit suite only; no installed app/domain acceptance.','checks':checks,'expectedCases':expected,'actualRows':rows,'actualSummary':summaries,'binary':binary,'actualRecorderProcessExit':r.returncode,'logSha256':sha(OUT/(prefix+'.log')),'policySha256':sha(OUT/'declared-sdk32-policy.json'),'helperSha256':sha(__file__)}
 save(prefix+'-aggregate.json',agg);print(json.dumps({'label':prefix,'accepted':agg['accepted'],'actualRows':len(rows),'summary':summaries,'checks':checks}));return r.returncode or (0 if agg['accepted'] else 1)
if __name__=='__main__':sys.exit(main())
