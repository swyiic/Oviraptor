#!/usr/bin/env python3
"""Prepare a fresh evidence-only copy; never run Cargo/project code or alter originals."""
import datetime,hashlib,json,pathlib,re,shutil,tarfile
BASE=pathlib.Path('/workspace/validation/agent-first-20261016');N=BASE/'native-next-stage';STAGE=N/'phase5-fixed-01';OUT=N/'native-evidence-delivery-phase5-v1'
def sha(p):
 with pathlib.Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def load(p):return json.loads(pathlib.Path(p).read_text())
def save(p,x):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(x,indent=2)+'\n')
def utc():return datetime.datetime.now(datetime.timezone.utc).isoformat()
assert not OUT.exists() and not OUT.with_suffix('.tar.gz').exists()
indices=[(STAGE/'phase5-focused-and-dev-evidence-index.json','027e4ed97b4c5a2c0b8cce95bff60c267b9218178c23fca25de878f223c6b5ef'),(N/'phase5/phase5-failed-check-evidence-index.json','926c889c30bed6f2c4557ef0664c124225352b9c0c9d4c671b2e7e5cb9c1c810')]
original_refs={};index_refs=[]
for p,expected in indices:
 assert sha(p)==expected,str(p)
 index_refs.append({'path':str(p),'sha256':expected,'bytes':p.stat().st_size})
 for r in load(p)['references']:
  q=pathlib.Path(r['path']);assert q.is_file() and not q.is_symlink() and q.stat().st_size==r['bytes'] and sha(q)==r['sha256'],str(q);original_refs[str(q)]=r
 original_refs[str(p)]={'path':str(p),'sha256':sha(p),'bytes':p.stat().st_size}
policy=load(STAGE/'declared-main-policy.json')
for r in policy['ownerManifestReferences']:
 p=pathlib.Path(r['path']);assert sha(p)==r['sha256'];original_refs[str(p)]={**r,'bytes':p.stat().st_size}
contract=N/'phase5-delivery-ui-handoff-contract-review.json';original_refs[str(contract)]={'path':str(contract),'sha256':sha(contract),'bytes':contract.stat().st_size}
OUT.mkdir(mode=0o700);copied=[];omitted=[]
for s,r in sorted(original_refs.items()):
 p=pathlib.Path(s);assert p.is_relative_to(BASE)
 if p.name.endswith(('.tar.gz','.tar','.db','.sqlite','.sqlite3')):
  omitted.append({**r,'bodyIncluded':False,'existedAndHashMatchedAtDelivery':True});continue
 dest=OUT/'originals'/p.relative_to(BASE);dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,dest);assert sha(dest)==r['sha256'];copied.append({**r,'packagePath':str(dest.relative_to(OUT)),'byteExact':True})
agg=load(STAGE/'phase5-focused-02-aggregate.json');sdk=load(STAGE/'phase5-sdk32-01-aggregate.json');sel=load(STAGE/'phase5-focused-02-selection.json');assert agg['accepted'] and sdk['accepted'] and sel['actualCaseCount']==4006 and sel['uniqueSelectedCount']==362
actual_names=re.findall(r'^(.+): test$',(STAGE/'phase5-focused-02-list.log').read_text(),re.M);assert len(actual_names)==len(set(actual_names))==4006
for label,names in [('actual-listed-4006-case-names',actual_names),('selected-core-347-case-names',sel['core']),('selected-paid-15-case-names',sel['paid']),('original-seven-ignored-case-names',sel['originalSevenIgnored']),('new-64-case-names',policy['expectedAdded']),('sdk-unit-32-case-names',sdk['expectedCases'])]:
 p=OUT/'derived'/(label+'.txt');p.parent.mkdir(exist_ok=True);p.write_text('\n'.join(names)+'\n')
frozen=load(STAGE/'frozen-native-inputs.json');assert len(frozen['nativeInputs'])==1820
save(OUT/'derived/recorded-native-input-fingerprints.json',{'recordedAtUtc':frozen['utc'],'recordedSourceHead':frozen['head'],'nativeInputCount':1820,'nativeInputs':frozen['nativeInputs'],'originalSnapshotPath':str(STAGE/'frozen-native-inputs.json'),'originalSnapshotSha256':sha(STAGE/'frozen-native-inputs.json'),'scope':'Recorded exact accepted native input bytes, not a whole-tree/final branch claim.'})
# Bodies stay outside this delivery. Observe preserved/current artifacts read-only.
observed={}
for p in load(STAGE/'initial-environment-plan.json')['protectedBinaries']:
 observed[p['path']]=p['sha256']
ret=load(STAGE/'phase5-passed-binary-and-source-retention-receipt.json')
for key in ['originalBinary','independentRegularCopy']:observed[ret[key]]=ret['binarySha256']
for r in load(STAGE/'phase5-dev-binaries-retention-receipt.json')['artifacts']:
 observed[r['original']]=r['sha256'];observed[r['independentRegularCopy']]=r['sha256']
sdkret=load(STAGE/'sdk32-retention-receipt.json');observed[sdkret['original']]=sdkret['sha256'];observed[sdkret['independentRegularCopy']]=sdkret['sha256']
artifacts=[]
for name,expected in sorted(observed.items()):
 p=pathlib.Path(name);assert p.is_file() and not p.is_symlink() and sha(p)==expected,name;artifacts.append({'path':name,'sha256':expected,'bytes':p.stat().st_size,'bodyIncluded':False,'existedAndHashMatchedAtDelivery':True})
save(OUT/'derived/protected-and-current-artifact-observation.json',{'observedAtUtc':utc(),'artifacts':artifacts,'count':len(artifacts),'scope':'Read-only hash observation. Future authorized builds may replace live output paths; independent retained copies preserve bytes. No artifact body included.'})
error=load(N/'phase5/phase5-all-targets-05-actual-tool-and-error-receipt.json');actual=load(STAGE/'phase5-actual-tool-and-acceptance-receipt.json')
summary={'createdAtUtc':utc(),'scope':'Default-feature native compile/dev-link and362 focused acceptance, plus separate32SDK units. This evidence copy performs no new test or compile.','firstCheck':{'actualExit':101,'seconds':77.426,'error':'E0277 usize:ToSql at scanning/native_service_sources.rs628; duplicate lib/libtest report, one source location.','rawLog':'originals/native-next-stage/phase5/phase5-all-targets-05.log','all1820NativeAnd21ProtectedUnchanged':True},'authorizedFix':'Only one Source file changed: checked i64::try_from for both bounded SQL usize values. SQL, limits, receipts and assertions preserved; original red remains in its own sealed index.','allTargets':{'actualExit':0,'seconds':76.290},'sdk':{'actualPassed':32,'failed':0,'ignored':0,'filtered':0,'harnessSeconds':0.51,'original31NamesPreserved':True},'devBins':load(STAGE/'phase5-dev-binaries-02.json')['artifacts'],'libNoRun':{'actualExit':0,'seconds':431.264,'optLevel':1,'debugAssertions':True,'overflowChecks':True,'defaultFeatures':True},'actualUniqueList':4006,'prior3942AllNamesPreserved':True,'all64NewCaseNamesActuallySelectedAndPassed':True,'originalSevenIgnoredInventoryExactlyPreserved':True,'core':agg['core']['summary'],'paid':agg['paid']['summary'],'uniqueFocusedExecutedAndPassed':362,'actualFocusedToolExit':0,'full4006PassedClaim':False,'fullExecuted':False,'releaseExecuted':False,'nativeInputs':1820,'protectedArtifacts':21,'allCommandAndFinalGuardsMatched':True,'actualToolReceipts':'originals/native-next-stage/phase5-fixed-01/phase5-actual-tool-and-acceptance-receipt.json','uiHandoffContractReview':'originals/native-next-stage/phase5-delivery-ui-handoff-contract-review.json','newBrowser06Or18AcceptanceClaim':False,'remaining':['Current full4006 library suite has not been executed.','Parent-reported known Source ancestor-Git gap and original-scale Source staging remain follow-up scope.','New ui.handoff declaration requires fresh normal package/install/current grants and a newly frozen browser run; no browser result is inferred from static SDK review.','Current full-service release size/installation/final whole-tree parity are not established by these dev/focused results.'],'binaryDatabaseAndSourceTarBodiesIncluded':False,'originalIndicesChanged':False,'newCargoOrNativeExecution':False}
save(OUT/'summary.json',summary)
save(OUT/'excluded-artifact-body-references.json',{'sourceArchives':omitted,'protectedAndCurrentBinaries':'derived/protected-and-current-artifact-observation.json','binariesDatabaseAndSourceTarBodiesCopied':False})
(OUT/'README.md').write_text('''# Phase5 native evidence delivery\n\nThis evidence-only directory is prepared outside the repository for a later reviewable\ndocumentation commit. It contains no executable binary, database, or source archive body.\n\nRead `summary.json` first. Actual default-feature library list4006 is not full4006 passed:\nonly the explicitly selected347core +15original paid neighbors ran and all362passed.\nThe separate SDK suite actually passed32/32. Original3942 names and7ignored inventory\nwere preserved. The initial real E0277/exit101 and checked-conversion retry are both\npresent. Current dev process links passed clean dependency inspection; browser, full,\nrelease, complete Source staging and known-ancestor Git paths remain separate scope.\n\n`originals/` contains byte-exact selected sealed evidence and required owner/baseline\nreferences. The original71-reference accepted index remains unchanged. Native1820\nfingerprints, plain selected/listed case names, actual exits/raw logs, and read-only\nprotected/current artifact path/hash observations are included. Large bodies stay at\nthe recorded cloud paths; absolute paths do not imply those binaries are in this package.\n\n`phase5-delivery-ui-handoff-contract-review.json` records static source/compiled-input\nproof that a valid permission may be declared without becoming a tool/action. Current\nbinaries can be copied into a newly validated package without Cargo for the page-only\nui.handoff declaration. It does not grant business authority or claim a successful new\nbrowser run; normal package/install/current-project grants and fresh verification apply.\n\nRun `python3 verify-delivery.py` after copying this directory. This only hashes evidence\nfiles and checks the closed file set; it invokes no Cargo/native/project tests. Historical\nhelpers in originals are audit provenance, not instructions or authorization for a new run.\n''')
verify=r'''#!/usr/bin/env python3
import hashlib,json
from pathlib import Path
r=Path(__file__).resolve().parent;x=json.loads((r/'delivery-index.json').read_text());errors=[]
for f in x['files']:
 p=r/f['path']
 if not p.is_relative_to(r) or not p.is_file() or p.is_symlink():errors.append(f['path']+': absent/nonregular');continue
 if p.stat().st_size!=f['bytes'] or hashlib.sha256(p.read_bytes()).hexdigest()!=f['sha256']:errors.append(f['path']+': size/hash mismatch')
actual={str(p.relative_to(r)) for p in r.rglob('*') if p.is_file()};expected={f['path'] for f in x['files']}|{'delivery-index.json'}
if actual!=expected:errors.append('file path set mismatch')
for f in x['byteExactOriginals']:
 if hashlib.sha256((r/f['packagePath']).read_bytes()).hexdigest()!=f['sha256']:errors.append(f['packagePath']+': original mismatch')
print(json.dumps({'accepted':not errors,'filesChecked':len(x['files']),'originalCopiesChecked':len(x['byteExactOriginals']),'errors':errors,'cargoOrNativeOrProjectTestsInvoked':False},indent=2));raise SystemExit(1 if errors else 0)
'''
(OUT/'verify-delivery.py').write_text(verify);shutil.copyfile(pathlib.Path(__file__),OUT/pathlib.Path(__file__).name)
files=[{'path':str(p.relative_to(OUT)),'bytes':p.stat().st_size,'sha256':sha(p)} for p in sorted(OUT.rglob('*')) if p.is_file()]
save(OUT/'delivery-index.json',{'schemaVersion':1,'createdAtUtc':utc(),'scope':summary['scope'],'sealedOriginalIndices':index_refs,'original71ReferencesAnd19FailedPhaseReferencesVerified':True,'byteExactOriginals':copied,'omittedLargeBodies':omitted,'files':files,'fileCountExcludingThisIndex':len(files),'binaryDatabaseAndSourceTarBodiesIncluded':False,'originalIndicesModified':False,'newCargoOrNativeExecution':False})
archive=OUT.with_suffix('.tar.gz')
with tarfile.open(archive,'w:gz',compresslevel=9) as tf:tf.add(OUT,arcname=OUT.name)
receipt={'createdAtUtc':utc(),'directory':str(OUT),'deliveryIndex':str(OUT/'delivery-index.json'),'deliveryIndexSha256':sha(OUT/'delivery-index.json'),'archive':str(archive),'archiveSha256':sha(archive),'archiveBytes':archive.stat().st_size,'filesIncludingIndex':len(files)+1,'byteExactOriginalCopies':len(copied),'allSealedOriginalReferenceHashesVerified':True,'protectedAndCurrentArtifactPathsVerified':len(artifacts),'excludedSourceArchiveBodies':len(omitted),'repositoryOrOriginalEvidenceModified':False,'cargoOrNativeExecutionInvoked':False};save(N/'native-evidence-delivery-phase5-v1-receipt.json',receipt);print(json.dumps(receipt,indent=2))
