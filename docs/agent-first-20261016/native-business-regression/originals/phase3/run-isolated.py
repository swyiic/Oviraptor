#!/usr/bin/env python3
"""Record one actual command against native source hashes without claiming JS-owner freeze."""
import datetime,hashlib,json,os,pathlib,re,shutil,subprocess,sys,time
ROOT=pathlib.Path('/workspace/Oviraptor-agent-first-20261016');OUT=pathlib.Path(__file__).resolve().parent
PROTECTED=json.loads((OUT/'initial-environment-plan.json').read_text())['protectedBinaries']
def sha(path):
 with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()
def utc():return datetime.datetime.now(datetime.timezone.utc).isoformat()
def save(path,value):
 with path.open('x') as stream:json.dump(value,stream,indent=2);stream.write('\n')
def snapshot():
 native={}
 roots=['src-tauri/src','src-tauri/tests','src-tauri/capabilities','packages/native-app-sdk/src']
 files=[ROOT/'src-tauri'/p for p in ['Cargo.toml','Cargo.lock','build.rs','tauri.conf.json']]
 files.extend(ROOT/'packages/native-app-sdk'/p for p in ['Cargo.toml','Cargo.lock','README.md'])
 for scope in roots:
  p=ROOT/scope
  if p.exists():files.extend(p.rglob('*'))
 for p in files:
  if p.is_symlink():raise RuntimeError('Native source symlink is not allowed: '+str(p.relative_to(ROOT)))
  if p.is_file():native[str(p.relative_to(ROOT))]=sha(p)
 return {'utc':utc(),'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'gitStatus':subprocess.check_output(['git','status','--porcelain=v1','--untracked-files=all'],cwd=ROOT,text=True).splitlines(),'nativeInputs':native,'protectedBinaries':[{'path':r['path'],'actualSha256':sha(pathlib.Path(r['path'])),'expectedSha256':r['sha256']} for r in PROTECTED]}
def main():
 label,*argv=sys.argv[1:]
 if not argv or not re.fullmatch('[A-Za-z0-9][A-Za-z0-9._-]*',label):raise RuntimeError('Fresh plain label and argv required')
 if list(OUT.glob(label+'*')):raise RuntimeError('Evidence prefix already exists')
 if shutil.disk_usage(ROOT).free<1073741824:raise RuntimeError('Less than1GiB free; stop without cleaning old evidence')
 before=snapshot();save(OUT/(label+'-before.json'),before);started=utc();tick=time.monotonic();disk_before=shutil.disk_usage(ROOT).free
 with (OUT/(label+'.log')).open('x') as stream:completed=subprocess.run(argv,cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT)
 finished=utc();after=snapshot();save(OUT/(label+'-after.json'),after)
 changes={'added':sorted(after['nativeInputs'].keys()-before['nativeInputs'].keys()),'removed':sorted(before['nativeInputs'].keys()-after['nativeInputs'].keys()),'changed':sorted(n for n in before['nativeInputs'].keys()&after['nativeInputs'].keys() if before['nativeInputs'][n]!=after['nativeInputs'][n])}
 checks={'actualCommandExitZero':completed.returncode==0,'nativeSourceHashesAndPathSetUnchanged':not any(changes.values()),'allProtectedBinariesPreserved':all(r['actualSha256']==r['expectedSha256'] for snap in [before,after] for r in snap['protectedBinaries'])}
 result={'label':label,'worktree':str(ROOT),'argv':argv,'startedAtUtc':started,'finishedAtUtc':finished,'durationSeconds':round(time.monotonic()-tick,3),'actualCommandExitCode':completed.returncode,'sourceHeadBefore':before['head'],'sourceHeadAfter':after['head'],'gitStatusBefore':before['gitStatus'],'gitStatusAfter':after['gitStatus'],'nativeInputCountBefore':len(before['nativeInputs']),'nativeInputCountAfter':len(after['nativeInputs']),'nativeChanges':changes,'checks':checks,'acceptedForNativeSource':all(checks.values()),'scope':'Only native source freeze asserted; independent JS owners may still edit other files. No whole-tree or clean-commit acceptance claimed.','diskFreeBytesBefore':disk_before,'diskFreeBytesAfter':shutil.disk_usage(ROOT).free,'environment':{k:os.environ.get(k) for k in ['RUSTC_WORKSPACE_WRAPPER','CARGO_TARGET_DIR','CARGO_INCREMENTAL','CARGO_BUILD_JOBS','CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_TEST_DEBUG','CARGO_PROFILE_TEST_OPT_LEVEL','CARGO_PROFILE_TEST_DEBUG_ASSERTIONS','CARGO_PROFILE_TEST_OVERFLOW_CHECKS','OVIRAPTOR_TEST_NO_SANDBOX','CHROME_BINARY']},'log':str(OUT/(label+'.log')),'logSha256':sha(OUT/(label+'.log')),'beforeSnapshotSha256':sha(OUT/(label+'-before.json')),'afterSnapshotSha256':sha(OUT/(label+'-after.json')),'recorderSha256':sha(pathlib.Path(__file__).resolve())}
 save(OUT/(label+'-result.json'),result);print(json.dumps({k:result[k] for k in ['label','durationSeconds','actualCommandExitCode','sourceHeadBefore','sourceHeadAfter','nativeInputCountBefore','nativeInputCountAfter','nativeChanges','checks','acceptedForNativeSource','diskFreeBytesAfter','log','logSha256']}));return completed.returncode or (0 if result['acceptedForNativeSource'] else 1)
if __name__=='__main__':sys.exit(main())
