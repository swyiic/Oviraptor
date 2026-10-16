#!/usr/bin/env python3
import argparse,datetime,hashlib,json,pathlib,re,runpy,subprocess,sys
OUT=pathlib.Path(__file__).resolve().parent;ROOT=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
def sha(p):
 with pathlib.Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(n,d):
 with (OUT/n).open('x') as f:json.dump(d,f,indent=2);f.write('\n')
def command(label,args):
 argv=[sys.executable,str(OUT/'run-isolated.py'),label,*args];started=datetime.datetime.now(datetime.timezone.utc).isoformat();result=subprocess.run(argv,cwd=ROOT)
 save(label+'-independent-receipt.json',{'argv':argv,'startedAtUtc':started,'finishedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'actualProcessExitCode':result.returncode})
 return result.returncode,(OUT/(label+'.log')).read_text()
def listed(text):
 rows=re.findall(r'^(.+): test$',text,re.M);assert rows and len(rows)==len(set(rows));return rows
def validate(text,selected,total):
 rows=re.findall(r'^test (.+) \.\.\. (ok|FAILED|ignored)$',text,re.M)
 summaries=re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in ([0-9.]+)s',text)
 checks={'allSelectedCasesExactlyOnce':len(rows)==len(selected)==len({n for n,s in rows}) and {n for n,s in rows}==set(selected),'allSelectedPassed':all(s=='ok' for n,s in rows),'oneCompleteSummary':len(summaries)==1}
 summary=None
 if len(summaries)==1:
  status,passed,failed,ignored,measured,filtered,seconds=summaries[0];summary={'status':status,'passed':int(passed),'failed':int(failed),'ignored':int(ignored),'measured':int(measured),'filtered':int(filtered),'harnessSeconds':float(seconds)}
  checks['actualSummaryMatchesExpected']=status=='ok' and tuple(map(int,[passed,failed,ignored,measured,filtered]))==(len(selected),0,0,0,total-len(selected))
 return {'rows':rows,'summary':summary,'checks':checks,'accepted':all(checks.values())}
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--build-prefix',required=True);parser.add_argument('--result-prefix',required=True);args=parser.parse_args();prefix=args.result_prefix
 assert re.fullmatch('[A-Za-z0-9][A-Za-z0-9._-]*',prefix) and not list(OUT.glob(prefix+'*'))
 build=json.loads((OUT/(args.build_prefix+'-result.json')).read_text());assert build['acceptedForNativeSource']
 expected_source=json.loads((OUT/(args.build_prefix+'-after.json')).read_text());actual_source=runpy.run_path(str(OUT/'run-isolated.py'))['snapshot']();assert expected_source['nativeInputs']==actual_source['nativeInputs']
 policy=json.loads((OUT/'declared-main-policy.json').read_text());proofs=[]
 for ref in policy['ownerManifestReferences']:
  assert sha(pathlib.Path(ref['path']))==ref['sha256']
 for path,expected_sha in policy['finalOwnedNativeHashes'].items():
  actual=sha(ROOT/path);assert actual==expected_sha,path;proofs.append({'path':path,'sha256':actual})
 artifacts=[]
 for line in (OUT/(args.build_prefix+'.log')).read_text().splitlines():
  try:item=json.loads(line)
  except ValueError:continue
  if item.get('reason')=='compiler-artifact' and item.get('target',{}).get('name')=='oviraptor_lib' and item.get('executable'):artifacts.append(item)
 assert len(artifacts)==1
 artifact=artifacts[0];profile=artifact['profile'];binary=pathlib.Path(artifact['executable']);assert artifact['target']['src_path']==str(ROOT/'src-tauri/src/lib.rs') and artifact['features']==[]
 assert profile['opt_level']=='1' and profile['test'] and profile['debug_assertions'] and profile['overflow_checks']
 assert binary.is_file() and not binary.is_symlink() and binary.resolve().is_relative_to('/workspace/asset-accuracy-target/debug/deps')
 binary_sha=sha(binary);save(prefix+'-artifact.json',{'artifact':artifact,'binarySha256':binary_sha,'bytes':binary.stat().st_size,'buildSourceHead':build['sourceHeadBefore'],'nativeInputsComparedToBuild':True,'declaredPolicySha256':sha(OUT/'declared-main-policy.json'),'actualOwnerNativeHashes':proofs})
 code,text=command(prefix+'-list',[str(binary),'--list']);assert code==0;names=listed(text);old=listed(pathlib.Path(policy['baselineActualList']).read_text());assert len(names)==policy['expectedActualCaseCount'] and len(old)==policy['baselineCount'] and set(old).issubset(names) and set(names)-set(old)==set(policy['expectedAdded'])
 assert len(old)==3942 and len(set(names)-set(old))==64
 code,text=command(prefix+'-ignored-list',[str(binary),'--list','--ignored']);assert code==0;ignored=listed(text);original_ignored=listed(pathlib.Path(policy['originalIgnoredList']).read_text());assert len(ignored)==7 and set(ignored)==set(original_ignored)
 core=sorted(set(policy['mandatoryOldFocused'])|set(policy['expectedAdded']));paid=sorted(n for n in names if n.startswith(policy['separatePaidNeighborhoodPrefix']));assert set(paid)==set(policy['expectedPaidNames']) and len(core)==policy['expectedCoreCount'] and len(paid)==policy['expectedPaidNeighbors'] and not set(core)&set(paid) and set(core+paid).issubset(names) and not set(core+paid)&set(ignored)
 save(prefix+'-selection.json',{'actualCaseCount':len(names),'old3942AllPreserved':True,'all64NewCasesSelected':True,'originalSevenIgnored':ignored,'core':core,'coreThreads':4,'paid':paid,'paidThreads':8,'uniqueSelectedCount':len(core)+len(paid),'binaryPath':str(binary),'binarySha256':binary_sha})
 base=[sys.executable,str(ROOT/'tools/run_with_subreaper.py'),str(binary)]
 code,text=command(prefix+'-core',base+['--exact',*core,'--test-threads=4']);core_result=validate(text,core,len(names));save(prefix+'-core-parsed-result.json',core_result)
 aggregate={'accepted':False,'scope':'Focused default-feature mainlibrary native acceptance only. No complete library suite or whole-tree acceptance claimed. Historical8thread failure remains recorded; deterministic clock snapshot mechanism and fix are covered without claiming the exact historical cause.','actualCaseCount':len(names),'baseline3942Preserved':True,'all64NewSelected':True,'originalSevenIgnoredPreserved':True,'uniqueSelectedCount':362,'core':core_result,'binarySha256Before':binary_sha,'binarySha256After':sha(binary),'helperSha256':sha(__file__)}
 if code or not core_result['accepted']:
  aggregate['actualCoreExitCode']=code;save(prefix+'-aggregate.json',aggregate);return code or 1
 code,text=command(prefix+'-paid-neighborhood',base+['--exact',*paid,'--test-threads=8']);paid_result=validate(text,paid,len(names));save(prefix+'-paid-parsed-result.json',paid_result);aggregate.update(paid=paid_result,actualCoreExitCode=0,actualPaidExitCode=code,binarySha256After=sha(binary));aggregate['accepted']=code==0 and paid_result['accepted'] and sha(binary)==binary_sha;save(prefix+'-aggregate.json',aggregate)
 print(json.dumps({'accepted':aggregate['accepted'],'actualTotalCases':len(names),'core':core_result['summary'],'paid':paid_result['summary'],'uniqueSelected':362}));return code or (0 if aggregate['accepted'] else 1)
if __name__=='__main__':sys.exit(main())
