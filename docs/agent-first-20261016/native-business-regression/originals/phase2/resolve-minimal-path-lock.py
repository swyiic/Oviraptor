#!/usr/bin/env python3
"""Authorized offline lock resolution: add only the already declared local SDK path package."""
import datetime,hashlib,json,pathlib,runpy,subprocess,sys,time,tomllib
OUT=pathlib.Path(__file__).resolve().parent;ROOT=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
def sha(p):
 with p.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()
def save(path,value):
 with path.open('x') as stream:json.dump(value,stream,indent=2);stream.write('\n')
def main():
 label='phase2-path-lock-resolution-01';assert not list(OUT.glob(label+'*'))
 recorder=runpy.run_path(str(OUT/'run-isolated.py'));before=recorder['snapshot']();save(OUT/(label+'-before.json'),before)
 lock=ROOT/'src-tauri/Cargo.lock';old_bytes=lock.read_bytes();(OUT/(label+'-Cargo.lock.before')).write_bytes(old_bytes);old=tomllib.loads(old_bytes.decode())
 argv=['cargo','metadata','--manifest-path','src-tauri/Cargo.toml','--offline','--format-version','1'];started=datetime.datetime.now(datetime.timezone.utc).isoformat();tick=time.monotonic()
 with (OUT/(label+'.log')).open('x') as stream:result=subprocess.run(argv,cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT)
 finished=datetime.datetime.now(datetime.timezone.utc).isoformat();after=recorder['snapshot']();save(OUT/(label+'-after.json'),after)
 new_bytes=lock.read_bytes();(OUT/(label+'-Cargo.lock.after')).write_bytes(new_bytes);new=tomllib.loads(new_bytes.decode())
 key=lambda entry:(entry['name'],entry['version'],entry.get('source'))
 oldp={key(x):x for x in old['package']};newp={key(x):x for x in new['package']};added=set(newp)-set(oldp);removed=set(oldp)-set(newp);changed=[k for k in set(oldp)&set(newp) if oldp[k]!=newp[k]]
 sdk=('oviraptor-native-app-sdk','2.0.0',None);roots=[k for k in oldp if k[0]=='oviraptor'];assert len(roots)==1;root=roots[0]
 root_old={k:v for k,v in oldp[root].items() if k!='dependencies'};root_new={k:v for k,v in newp[root].items() if k!='dependencies'}
 dep_before=set(oldp[root].get('dependencies',[]));dep_after=set(newp[root].get('dependencies',[]));dep_added=dep_after-dep_before
 new_entry=newp.get(sdk,{})
 allowed_changes=(added=={sdk} and not removed and changed==[root] and root_old==root_new and dep_before.issubset(dep_after) and len(dep_added)==1 and next(iter(dep_added)).split()[0]=='oviraptor-native-app-sdk' and set(new_entry)=={'name','version','dependencies'} and {x.split()[0] for x in new_entry.get('dependencies',[])}=={'serde','serde_json','sha2'})
 # A pre-existing exact path addition is acceptable as a documented no-op;
 # this helper still may not alter any pinned registry package.
 no_change=(old_bytes==new_bytes and sdk in newp)
 a=before['nativeInputs'];b=after['nativeInputs'];source_changed=sorted(k for k in set(a)|set(b) if a.get(k)!=b.get(k))
 protected=all(x['actualSha256']==x['expectedSha256'] for snap in [before,after] for x in snap['protectedBinaries'])
 checks={'actualMetadataExitZero':result.returncode==0,'onlyExpectedPathPackageAddedOrAlreadyPresent':allowed_changes or no_change,'nativeSourceExceptAuthorizedLockUnchanged':set(source_changed).issubset({'src-tauri/Cargo.lock'}),'allProtectedBinariesPreserved':protected,'noLockFormatOrOtherTopLevelChange':{k:v for k,v in old.items() if k!='package'}=={k:v for k,v in new.items() if k!='package'}}
 receipt={'argv':argv,'startedAtUtc':started,'finishedAtUtc':finished,'durationSeconds':time.monotonic()-tick,'actualProcessExitCode':result.returncode,'notClaimedLocked':True,'authorization':'Root explicitly allowed only offline minimal local SDK path addition after combined freeze. No dependency upgrade intended.','addedPackages':sorted(added),'removedPackages':sorted(removed),'changedExistingPackages':changed,'rootDependencyAdded':sorted(dep_added),'nativeChanges':source_changed,'checks':checks,'accepted':all(checks.values()),'oldLockSha256':hashlib.sha256(old_bytes).hexdigest(),'newLockSha256':hashlib.sha256(new_bytes).hexdigest(),'logSha256':sha(OUT/(label+'.log')),'helperSha256':sha(pathlib.Path(__file__))}
 save(OUT/(label+'-result.json'),receipt);print(json.dumps(receipt));return result.returncode or (0 if receipt['accepted'] else 1)
if __name__=='__main__':sys.exit(main())
