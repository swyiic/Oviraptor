#!/usr/bin/env python3
"""Run only after real focused acceptance to preserve that exact binary/input set."""
import argparse,datetime,hashlib,json,pathlib,runpy,shutil,tarfile
OUT=pathlib.Path(__file__).resolve().parent;ROOT=pathlib.Path('/workspace/Oviraptor-agent-first-20261016')
def sha(p):
 with pathlib.Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(p,d):
 with p.open('x') as f:json.dump(d,f,indent=2);f.write('\n')
def main():
 p=argparse.ArgumentParser();p.add_argument('--build-prefix',required=True);p.add_argument('--focused-prefix',required=True);a=p.parse_args()
 build=json.loads((OUT/(a.build_prefix+'-result.json')).read_text());agg=json.loads((OUT/(a.focused_prefix+'-aggregate.json')).read_text());art=json.loads((OUT/(a.focused_prefix+'-artifact.json')).read_text());assert build['acceptedForNativeSource'] and agg['accepted'] and agg['actualCaseCount']==4006 and agg['uniqueSelectedCount']==362
 binary=pathlib.Path(art['artifact']['executable']);assert sha(binary)==art['binarySha256']==agg['binarySha256After'];frozen=json.loads((OUT/'frozen-native-inputs.json').read_text());snap=runpy.run_path(str(OUT/'run-isolated.py'))['snapshot']();assert snap['nativeInputs']==frozen['nativeInputs'] and len(snap['nativeInputs'])==1820 and all(r['actualSha256']==r['expectedSha256'] for r in snap['protectedBinaries'])
 target=OUT/'retained-main-4006-focused-passed'/binary.name;target.parent.mkdir();shutil.copyfile(binary,target);assert target.stat().st_ino!=binary.stat().st_ino and target.stat().st_nlink==1 and sha(target)==sha(binary)
 archive=OUT/'passed-4006-native-source-1820.tar.gz'
 with tarfile.open(archive,'x:gz') as tf:
  for name,h in sorted(frozen['nativeInputs'].items()):
   source=ROOT/name;assert sha(source)==h,name;tf.add(source,arcname=name,recursive=False)
 with tarfile.open(archive,'r:gz') as tf:
  files={m.name:m for m in tf.getmembers()};assert set(files)==set(frozen['nativeInputs'])
  for name,h in frozen['nativeInputs'].items():
   m=files[name];assert m.isfile() and hashlib.sha256(tf.extractfile(m).read()).hexdigest()==h,name
 after=runpy.run_path(str(OUT/'run-isolated.py'))['snapshot']();assert snap['nativeInputs']==after['nativeInputs'] and all(r['actualSha256']==r['expectedSha256'] for r in after['protectedBinaries'])
 receipt={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'nativeInputs':1820,'actualListedCases':4006,'focusedPassed':362,'originalBinary':str(binary),'independentRegularCopy':str(target),'binarySha256':sha(target),'binaryBytes':target.stat().st_size,'distinctInode':True,'sourceArchive':str(archive),'sourceArchiveSha256':sha(archive),'archiveBytes':archive.stat().st_size,'all1820ArchivedFilesExact':True,'sourceAndProtectedBeforeAfterMatch':True,'fullExecuted':False,'releaseExecuted':False,'scope':'Default-feature library focused native acceptance only; current whole-tree/final full/release acceptance remains pending.'}
 save(OUT/'phase5-passed-binary-and-source-retention-receipt.json',receipt);print(json.dumps(receipt))
if __name__=='__main__':main()
