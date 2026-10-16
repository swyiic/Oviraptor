#!/usr/bin/env python3
"""Run the frozen native library suite while Node owners continue outside native scope."""
import argparse, datetime, hashlib, importlib.util, json, os, pathlib, re, shutil, subprocess, sys, time
OUT=pathlib.Path(__file__).resolve().parent
ROOT=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
NATIVE_COMMIT='3d2eca1cc2aa3f0c85a51bdf8fe125e0a7659d15'
def utc():return datetime.datetime.now(datetime.timezone.utc).isoformat()
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def load(n):return json.loads((OUT/n).read_text())
def save(n,v):
 with (OUT/n).open('x') as f:json.dump(v,f,indent=2);f.write('\n')
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
recorder=module('native_recorder',OUT/'run-isolated.py');snap=module('broad_snapshot',OUT/'snapshot-module.py')
def diff(a,b):return {'before':len(a),'after':len(b),'added':sorted(b.keys()-a.keys()),'removed':sorted(a.keys()-b.keys()),'changed':sorted(k for k in a.keys()&b.keys() if a[k]!=b[k])}
def main():
 args=argparse.ArgumentParser();args.add_argument('--execute',action='store_true');v=args.parse_args()
 if not v.execute:raise RuntimeError('Explicit --execute required')
 repeat=load('paid-neighborhood-repeat-10/repeat-result.json')
 if not repeat['accepted'] or repeat['roundsCompleted']!=10 or repeat['uniqueCaseCount']!=15 or repeat['totalCaseExecutions']!=150:raise RuntimeError('Ten original15-case8thread rounds must really pass before full4')
 prefix='service-native-full-03-threads4'
 if list(OUT.glob(prefix+'*')):raise RuntimeError('Fresh full evidence prefix required')
 if not load('service-focused-aggregate.json')['accepted'] or not load('source-preflight-aggregate.json')['accepted']:raise RuntimeError('Focused prerequisites failed')
 a=load('service-library-artifact.json');binary=OUT/'retained-113a-full-failure/oviraptor_lib-75f6df7542c7ea38';profile=a['cargoArtifact']['profile'];contract=load('case-migration-and-ignore-contract.json')
 if binary.is_symlink() or not binary.is_file() or binary.resolve()!=OUT/'retained-113a-full-failure/oviraptor_lib-75f6df7542c7ea38' or not os.access(binary,os.X_OK):raise RuntimeError('Unexpected binary')
 if sha(binary)!=a['binarySha256'] or a['cargoArtifact']['features']!=[] or str(profile['opt_level'])!='1' or not all(profile.get(k) is True for k in ['test','debug_assertions','overflow_checks']):raise RuntimeError('Binary/profile differs from accepted focused artifact')
 pre=recorder.snapshot();baseline=load('service-focused-01-after.json')
 if pre['nativeInputs']!=baseline['nativeInputs'] or len(pre['nativeInputs'])!=1774:raise RuntimeError('Native input hashes/pathset differ from accepted focused source')
 native_diff=subprocess.check_output(['git','diff',NATIVE_COMMIT,'--','src-tauri'],cwd=ROOT)
 if native_diff:raise RuntimeError('Native source differs from approved committed native baseline')
 if shutil.disk_usage(ROOT).free<1073741824:raise RuntimeError('Less than1GiB free; no cleanup authorized')
 listing=subprocess.run([str(binary),'--list'],cwd=ROOT,capture_output=True,text=True);(OUT/(prefix+'-list.txt')).write_text(listing.stdout+listing.stderr)
 ignored_list=subprocess.run([str(binary),'--list','--ignored'],cwd=ROOT,capture_output=True,text=True);(OUT/(prefix+'-ignored-list.txt')).write_text(ignored_list.stdout+ignored_list.stderr)
 names=[line[:-6] for line in listing.stdout.splitlines() if line.endswith(': test')];ignored=[line[:-6] for line in ignored_list.stdout.splitlines() if line.endswith(': test')];old_names=[line[:-6] for line in (OUT/'service-list-01.log').read_text().splitlines() if line.endswith(': test')]
 if listing.returncode or ignored_list.returncode or len(names)!=len(set(names)) or len(names)!=3841 or set(names)!=set(old_names) or len(ignored)!=7 or set(ignored)!=set(contract['actualIgnoredNames']):raise RuntimeError('Actual full list or original7ignored changed')
 before=snap.snapshot(binary);save(prefix+'-source-before.json',before)
 save(prefix+'-authorization-and-baseline.json',{'nativeCommittedBaseline':NATIVE_COMMIT,'fullCommandUnfiltered':True,'testThreads':4,'cgroupCpuMax':pathlib.Path('/sys/fs/cgroup/cpu.max').read_text().strip(),'comparisonPolicy':'Same immutable113a binary, original3841 cases/7ignored/assertions/TTL; new4thread run does not erase original8thread101 or establish a cause.','nativeFocusedSnapshotSha256':sha(OUT/'service-focused-01-after.json'),'nativePre':pre,'scope':'Native1774 file hashes/pathset frozen; independent Node/docs owners may edit/commit outside native scope. Whole repository final acceptance is not claimed.','actualListExitCode':listing.returncode,'actualIgnoredListExitCode':ignored_list.returncode,'actualListCount':len(names),'actualIgnoredNames':ignored,'runnerSha256':sha(pathlib.Path(__file__)),'snapshotHelperSha256':sha(OUT/'snapshot-module.py')})
 command=[sys.executable,'tools/run_with_subreaper.py',str(binary),'--test-threads=4'];started=utc();tick=time.monotonic()
 process=subprocess.run([sys.executable,str(OUT/'run-isolated.py'),prefix+'-command',*command],cwd=ROOT)
 finished=utc();after=snap.snapshot(binary);save(prefix+'-source-after.json',after);post=recorder.snapshot();save(prefix+'-native-post.json',post)
 receipt=load(prefix+'-command-result.json');text=(OUT/(prefix+'-command.log')).read_text(errors='replace')
 cases=re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?:[ \t].*)?$',text,re.M)
 summaries=re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in ([0-9.]+)s$',text,re.M)
 changes={k:diff(before[k],after[k]) for k in ['nativeInputs','auditInputs','trackedInputs']}
 checks={'guardedOuterExitZero':process.returncode==0,'actualTestCommandExitZero':receipt['actualCommandExitCode']==0,'native1774EqualsAcceptedFocusedPrePost':pre['nativeInputs']==post['nativeInputs']==baseline['nativeInputs'],'nativeGuardAccepted':receipt['acceptedForNativeSource'],'binaryUnchanged':sha(binary)==a['binarySha256'],'fourHistoricalBinariesPreserved':all(r['actualSha256']==r['expectedSha256'] for x in [pre,post] for r in x['protectedBinaries']),'exactListedCaseSetEachOnce':len(cases)==3841 and len({n for n,_ in cases})==3841 and {n for n,_ in cases}==set(names),'originalSevenIgnoredOnly':{n for n,s in cases if s=='ignored'}==set(ignored),'noFailedCases':not any(s=='FAILED' for _,s in cases),'oneStrictCompleteSummary':len(summaries)==1 and list(summaries[0][:6])==['ok','3834','0','7','0','0']}
 result={'acceptedForNativeSource':all(checks.values()),'checks':checks,'nativeCommittedBaseline':NATIVE_COMMIT,'scope':'Native-source library acceptance only. Node/docs may change outside the frozen1774 inputs; all-source changes are transparently reported below.','sourceHeadBefore':pre['head'],'sourceHeadAfter':post['head'],'command':command,'binary':str(binary),'binarySha256':a['binarySha256'],'profile':profile,'features':[],'testThreads':4,'cgroupCpuMax':pathlib.Path('/sys/fs/cgroup/cpu.max').read_text().strip(),'original8threadFullExitCode':101,'causeConclusion':'Unknown; 4thread comparison is not a production fix or erasure of original8thread failure.','startedAtUtc':started,'finishedAtUtc':finished,'durationSeconds':round(time.monotonic()-tick,3),'actualGuardedRunnerExitCode':process.returncode,'actualTestCommandExitCode':receipt['actualCommandExitCode'],'listedTests':len(names),'completedCases':len(cases),'passedCaseNames':[n for n,s in cases if s=='ok'],'failedCaseNames':[n for n,s in cases if s=='FAILED'],'ignoredCaseNames':[n for n,s in cases if s=='ignored'],'missingCaseNames':sorted(set(names)-{n for n,_ in cases}),'unexpectedCaseNames':sorted({n for n,_ in cases}-set(names)),'summary':summaries,'informationalBroaderSourceComparisons':changes,'logSha256':sha(OUT/(prefix+'-command.log')),'sourceBeforeSha256':sha(OUT/(prefix+'-source-before.json')),'sourceAfterSha256':sha(OUT/(prefix+'-source-after.json')),'diskFreeBytes':shutil.disk_usage(ROOT).free}
 save(prefix+'-aggregate.json',result);print(json.dumps({k:result[k] for k in ['acceptedForNativeSource','checks','startedAtUtc','finishedAtUtc','durationSeconds','actualTestCommandExitCode','listedTests','completedCases','summary','informationalBroaderSourceComparisons']}))
 return process.returncode or (0 if result['acceptedForNativeSource'] else 1)
if __name__=='__main__':raise SystemExit(main())
