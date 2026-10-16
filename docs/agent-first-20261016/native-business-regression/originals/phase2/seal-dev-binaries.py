#!/usr/bin/env python3
import argparse,datetime,hashlib,json,pathlib,re,runpy,subprocess
OUT=pathlib.Path(__file__).resolve().parent;ROOT=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
def sha(p):
 with pathlib.Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--build-prefix',required=True);p.add_argument('--result-name',required=True);args=p.parse_args()
 assert re.fullmatch('[A-Za-z0-9][A-Za-z0-9._-]*',args.result_name)
 build=json.loads((OUT/(args.build_prefix+'-result.json')).read_text());assert build['acceptedForNativeSource']
 expected=json.loads((OUT/(args.build_prefix+'-after.json')).read_text());now=runpy.run_path(str(OUT/'run-isolated.py'))['snapshot']();assert expected['nativeInputs']==now['nativeInputs']
 found={name:[] for name in ['oviraptor-scan-service','oviraptor-assets-service']}
 for line in (OUT/(args.build_prefix+'.log')).read_text().splitlines():
  try:item=json.loads(line)
  except ValueError:continue
  if item.get('reason')=='compiler-artifact' and item.get('target',{}).get('name') in found and item.get('executable'):found[item['target']['name']].append(item)
 artifacts=[]
 for name,items in found.items():
  assert len(items)==1,(name,len(items));artifact=items[0];profile=artifact['profile'];binary=pathlib.Path(artifact['executable'])
  assert artifact['target']['kind']==['bin'] and artifact['target']['src_path']==str(ROOT/f'src-tauri/src/bin/{name}.rs') and artifact['features']==[] and not profile['test'] and profile['opt_level']=='0'
  assert binary.is_file() and not binary.is_symlink() and binary.resolve().is_relative_to('/workspace/asset-accuracy-target/debug')
  before=sha(binary);env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'};commands=[]
  for argv in [['readelf','-d',str(binary)],['ldd',str(binary)]]:
   result=subprocess.run(argv,env=env,capture_output=True,text=True);commands.append({'argv':argv,'environment':env,'actualExitCode':result.returncode,'stdout':result.stdout,'stderr':result.stderr});assert result.returncode==0
  needed=re.findall(r'\(NEEDED\).*Shared library: \[([^\]]+)\]',commands[0]['stdout']);missing=[line.strip() for line in commands[1]['stdout'].splitlines() if 'not found' in line];private=any('/workspace/toolchain' in cmd['stdout'] for cmd in commands)
  artifacts.append({'name':name,'artifact':artifact,'path':str(binary),'bytes':binary.stat().st_size,'sha256Before':before,'sha256After':sha(binary),'dtNeeded':needed,'cleanEnvironmentMissingLibraries':missing,'privateSysrootRequiredOrResolved':private,'actualDependencyInspection':commands,'headlessCleanEnvironmentReady':not missing and not private})
  assert sha(binary)==before
 native=json.dumps(expected['nativeInputs'],sort_keys=True,separators=(',',':')).encode()
 data={'createdAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'sourceHeadBefore':build['sourceHeadBefore'],'sourceHeadAfter':build['sourceHeadAfter'],'nativeInputCount':len(expected['nativeInputs']),'nativeInputMapCanonicalSha256':hashlib.sha256(native).hexdigest(),'sourceProof':str(OUT/(args.build_prefix+'-after.json')),'sourceProofSha256':sha(OUT/(args.build_prefix+'-after.json')),'buildActualExitCode':build['actualCommandExitCode'],'buildNativeGuardsAccepted':True,'artifacts':artifacts,'scope':'Actual default-feature dev non-test process binaries. Dependency inspection and compiled source proof only; no smoke, release-size, installed-application or full business parity acceptance claimed.','helperSha256':sha(__file__)}
 with (OUT/args.result_name).open('x') as f:json.dump(data,f,indent=2);f.write('\n')
 print(json.dumps({'result':str(OUT/args.result_name),'sourceCount':len(expected['nativeInputs']),'bins':[{'name':x['name'],'path':x['path'],'sha256':x['sha256After'],'bytes':x['bytes'],'dtNeeded':x['dtNeeded'],'headlessCleanEnvironmentReady':x['headlessCleanEnvironmentReady']} for x in artifacts]}))
if __name__=='__main__':main()
