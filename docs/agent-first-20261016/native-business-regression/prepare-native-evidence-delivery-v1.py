#!/usr/bin/env python3
"""Read sealed native phase evidence; create a fresh, small audit delivery only."""
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tarfile

BASE = Path('/workspace/validation/agent-first-20261016/native-next-stage')
OUT = BASE / 'native-evidence-delivery-phase2-4-v1'
SCRIPT = Path(__file__).resolve()


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def read(p):
    return json.loads(p.read_text())


def write(p, value):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')


if OUT.exists() or OUT.with_suffix('.tar.gz').exists():
    raise SystemExit('Fresh delivery path is required; no overwrite permitted')
started = now()
indices = {}
verified_originals = []
for phase in ('phase2', 'phase3', 'phase4'):
    p = BASE / phase / f'{phase}-evidence-index.json'
    if not p.is_file() or p.is_symlink():
        raise SystemExit(f'Index must be a regular file: {p}')
    indices[phase] = read(p)
    for ref in indices[phase]['references']:
        q = Path(ref['path'])
        if not q.is_relative_to(BASE / phase) or not q.is_file() or q.is_symlink():
            raise SystemExit(f'Unexpected original reference: {q}')
        if q.stat().st_size != ref['bytes'] or sha(q) != ref['sha256']:
            raise SystemExit(f'Sealed original mismatch: {q}')
        verified_originals.append({'phase': phase, **ref})

OUT.mkdir(mode=0o700)
source_refs = []
omitted = []
for phase, index in indices.items():
    refs = [{'path': str(BASE / phase / f'{phase}-evidence-index.json'),
             'sha256': sha(BASE / phase / f'{phase}-evidence-index.json'),
             'bytes': (BASE / phase / f'{phase}-evidence-index.json').stat().st_size}] + index['references']
    for ref in refs:
        p = Path(ref['path'])
        if p.name.endswith(('.tar.gz', '.tar', '.db', '.sqlite', '.sqlite3')):
            omitted.append({**ref, 'bodyIncluded': False,
                            'reason': 'Large source archive or database bodies excluded from the lightweight delivery',
                            'authority': 'PARTIAL_FAILED_ATTEMPT' if p.name == 'passed-3942-native-source.tar.gz' else 'RECORDED_SOURCE_ARCHIVE',
                            'existedAndHashMatchedAtDelivery': True})
            continue
        target = OUT / 'originals' / p.relative_to(BASE)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(p, target)
        if sha(target) != ref['sha256']:
            raise SystemExit(f'Copy mismatch: {p}')
        source_refs.append({**ref, 'packagePath': str(target.relative_to(OUT)),
                            'copyIsByteExact': True})

# Explicit, plain-name selection/list files are derived only from recorded real --list output.
selections = {}
for phase, expected in [('phase2', 3938), ('phase3', 3942)]:
    folder = BASE / phase
    log = folder / f'{phase}-focused-01-list.log'
    names = re.findall(r'^(.+): test$', log.read_text(), re.M)
    if len(names) != expected or len(set(names)) != expected:
        raise SystemExit(f'Actual unique list mismatch: {phase}')
    selection = read(folder / f'{phase}-focused-01-selection.json')
    core, paid = selection['core'], selection['paid']
    if len(set(core + paid)) != selection['uniqueSelectedCount']:
        raise SystemExit(f'Duplicate selection: {phase}')
    if set(core + paid) - set(names):
        raise SystemExit(f'Unlisted selected case: {phase}')
    ignored = selection['originalSevenIgnored']
    if len(ignored) != 7 or len(set(ignored)) != 7:
        raise SystemExit(f'Ignored inventory mismatch: {phase}')
    d = OUT / 'derived' / phase
    d.mkdir(parents=True)
    for label, rows in [('actual-listed-case-names', names), ('selected-core-case-names', core),
                        ('selected-paid-neighborhood-case-names', paid), ('original-seven-ignored-case-names', ignored)]:
        (d / (label + '.txt')).write_text('\n'.join(rows) + '\n')
    selections[phase] = {'actualListed': expected, 'plannedCore': len(core), 'plannedPaid': len(paid),
                         'uniquePlannedSelection': len(core) + len(paid),
                         'allSelectedExistInActualList': True,
                         'paidActuallyExecuted': phase == 'phase3'}

# Reusable source fingerprints include the exact native input maps captured by the actual commands.
source_manifests = {}
for phase, snapshot_name in [
    ('phase2', 'phase2-focused-01-core-after.json'),
    ('phase3', 'phase3-focused-01-core-after.json'),
    ('phase4', 'phase4-assets-dev-build-01-after.json')]:
    p = BASE / phase / snapshot_name
    snap = read(p)
    inputs = snap['nativeInputs']
    if len(inputs) != 1810:
        raise SystemExit(f'Expected 1810 captured inputs: {p}')
    native_map = {'phase': phase, 'recordedAtUtc': snap['utc'], 'recordedSourceHead': snap['head'],
                  'nativeInputCount': len(inputs), 'nativeInputs': inputs,
                  'nativeMapCanonicalSha256': hashlib.sha256(json.dumps(inputs, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
                  'originalSnapshotPath': str(p), 'originalSnapshotSha256': sha(p),
                  'scope': 'Historical exact input bytes, not a claim about later whole-Git HEAD or source changes.'}
    write(OUT / 'derived' / phase / 'recorded-native-input-fingerprints.json', native_map)
    source_manifests[phase] = {k: v for k, v in native_map.items() if k != 'nativeInputs'}

# Verify current existence/hashes of the recorded phase4 protection inventory, without copying bodies.
protected = read(BASE / 'phase4' / 'phase4-assets-dev-build-01-after.json')['protectedBinaries']
protected_observed = []
for item in protected:
    p = Path(item['path'])
    st = p.lstat()
    actual = sha(p)
    if not p.is_file() or p.is_symlink() or actual != item['expectedSha256']:
        raise SystemExit(f'Protected artifact mismatch: {p}')
    protected_observed.append({'path': str(p), 'bytes': st.st_size, 'sha256': actual,
                               'expectedSha256': item['expectedSha256'], 'matches': True,
                               'bodyIncluded': False, 'observedAtUtc': now()})
latest = read(BASE / 'phase4' / 'phase4-actual-tool-and-artifact-retention-receipt.json')
for key in ['latestAssetsBinary', 'independentRegularCopy']:
    p = Path(latest[key]); st = p.lstat(); actual = sha(p)
    if not p.is_file() or p.is_symlink() or actual != latest['assetsBinarySha256']:
        raise SystemExit(f'Latest Assets artifact mismatch: {p}')
    protected_observed.append({'path': str(p), 'bytes': st.st_size, 'sha256': actual,
                               'expectedSha256': latest['assetsBinarySha256'], 'matches': True,
                               'bodyIncluded': False, 'observedAtUtc': now(),
                               'role': key})
write(OUT / 'derived' / 'protected-artifact-observation.json', {
    'observedAtUtc': now(), 'count': len(protected_observed), 'artifacts': protected_observed,
    'scope': 'Read-only current hash observation. Live output paths may be replaced by future authorized builds; retained independent copies preserve the recorded bytes.'})

failed = read(BASE / 'phase2' / 'phase2-focused-01-core-parsed-result.json')
fail_rows = [row[0] for row in failed['rows'] if row[1] == 'FAILED']
if len(fail_rows) != 3:
    raise SystemExit('Expected original three actual focused failures')
error_receipt = read(BASE / 'phase2' / 'phase2-all-targets-01-actual-tool-and-error-receipt.json')
if error_receipt['actualObservedToolExitCode'] != 101:
    raise SystemExit('Original compile red must be actual101')
phase3_core = read(BASE / 'phase3' / 'phase3-focused-01-core-parsed-result.json')
phase3_paid = read(BASE / 'phase3' / 'phase3-focused-01-paid-parsed-result.json')
summary = {
    'createdAtUtc': now(), 'scope': 'Audit delivery of historical native phase2 red, phase3 focused green, and phase4 compile/dev-link-only acceptance. No new Cargo/test execution.',
    'phase2': {
        'acceptedAsBatch': False,
        'compileFirstActualExit': 101,
        'compileError': 'E0308 Result<(), &str> versus Result<(), String>',
        'compileErrorLocations': error_receipt['uniqueErrorLocations'],
        'laterAllTargets': indices['phase2']['allTargets02'],
        'actualListAndSelection': selections['phase2'],
        'core': failed['summary'],
        'actualCoreToolExit': read(BASE / 'phase2' / 'phase2-first-focused-failure-and-retention-receipt.json')['actualObservedToolExitCode'],
        'failedCases': fail_rows,
        'paidNeighborhoodExecuted': False,
        'assetsRuntime': 'GTK/WebKit dependencies were genuinely present; old failed-runtime binary retained. Later headless seam belongs to phase3.',
        'fullExecuted': False,
        'rawFailureLog': 'originals/phase2/phase2-focused-01-core.log',
        'rawCompileErrorLog': 'originals/phase2/phase2-all-targets-01.log'},
    'phase3': {
        'actualAllTargets': indices['phase3']['allTargets'],
        'actualLibNoRun': indices['phase3']['defaultLibNoRun'],
        'actualListAndSelection': selections['phase3'],
        'core': phase3_core['summary'], 'paid': phase3_paid['summary'],
        'actualToolReceipt': read(BASE / 'phase3' / 'phase3-actual-tool-and-retention-receipt.json'),
        'fullExecuted': False, 'updatedReleaseBuilt': False,
        'doesNotCoverLaterArtifactOffsetTwoFileChange': True},
    'phase4': {**{k:v for k,v in indices['phase4'].items() if k != 'references'},
               'actualToolReceipt': latest,
               'newLibNoRunOrCasesExecuted': False, 'fullExecuted': False, 'releaseExecuted': False},
    'recordedSourceFingerprints': source_manifests,
    'originalIndicesVerified': True,
    'originalReferenceCountVerified': len(verified_originals),
    'cargoOrNativeTestsStartedWhileCreatingDelivery': False,
    'repositoryWritten': False,
    'binaryDatabaseAndSourceArchiveBodiesIncluded': False,
    'remainingAcceptance': ['Current-source final full library suite is pending a later root SOURCE FREEZE.',
                            'Current full service release size/link and installation acceptance are not inferred from the old bootstrap-only release.',
                            'Browser/package/UI results are separately owned and not inferred from check or clean ldd.']}
write(OUT / 'summary.json', summary)
write(OUT / 'omitted-large-artifact-body-references.json', {'observedAtUtc': now(), 'sourceArchives': omitted,
      'protectedArtifacts': 'derived/protected-artifact-observation.json',
      'noBinaryOrDatabaseCopied': True})

readme = '''# Native phase2–4 audit delivery

This is a lightweight, read-only evidence copy for a later Git documentation commit.
It contains no executable binary, SQLite database, or source tar body. Absolute paths in the
original receipts identify the original cloud environment; copied evidence paths are relative.

## Recorded result scope

- Phase2: first all-targets compile really exited101 (E0308). The later compile succeeded.
  Actual3938 library names were listed. Core279 executed:276passed,3failed. Paid15 was planned
  but did not execute after the core failure. The3panic bodies remain in the complete raw log.
  The GTK-linked Assets dev binary was a real runtime limitation, not installed acceptance.
- Phase3: all-targets and opt1 library no-run succeeded. Actual3942 names preserved3938 and
  added4. Core283 and original paid15 all passed, every selected case exactly once.
  This is298 focused cases, not the full3942 suite. It applies to the exact1810 recorded
  source inputs before the later2-file artifact wire change.
- Phase4: nextOffset is an integer end offset even on EOF. Only those2 source paths changed.
  New all-targets and actual Assets dev link succeeded. Clean dependency inspection found
  only4standard Linux libraries. No new library no-run/list/test/full/release was executed.
  Browser acceptance is separate. The old phase3 passed library binary remains preserved.

## Reading and verification

Start with `summary.json` and `delivery-index.json`. `originals/` contains byte-exact copies
of all regular small files referenced by the3sealed original indices, plus the3indices.
Original source tar bodies are omitted with their original SHA/size/path recorded.
The original indices are unchanged historical objects; their absolute references may name
excluded large originals. `delivery-index.json` maps copied bytes to those originals.
`derived/` contains plain case-name lists, exact recorded source fingerprint maps, and a
fresh read-only hash observation of19protected artifacts plus the2latest Assets paths.

Run `python3 verify-delivery.py` after copying this directory. It only hashes files and
checks the delivery index. It does not invoke Cargo, native binaries, or project tests.
No helper stored in `originals/` should be run as an acceptance command: they are historical,
may have obsolete source policies, and do not authorize a new run.

No new passing test count is created by this packaging step. Existing red logs, actual tool
exits, failed partial archive attempt, scope restrictions, source-HEAD changes during JS/docs
work, and excluded-runtime limitations remain visible. No source, handoff, prior evidence,
or protected binary was modified. Full/default library, release, and final whole-tree scope
must be established separately under the parent's next explicit source freeze.
'''
(OUT / 'README.md').write_text(readme)
verify = r'''#!/usr/bin/env python3
"""Verify only this evidence delivery; never run project code/tests."""
import hashlib,json
from pathlib import Path
root=Path(__file__).resolve().parent
idx=json.loads((root/'delivery-index.json').read_text())
errors=[]
for ref in idx['files']:
 p=root/ref['path']
 if not p.is_relative_to(root) or p.is_symlink() or not p.is_file():
  errors.append(ref['path']+': absent/nonregular'); continue
 h=hashlib.sha256(p.read_bytes()).hexdigest()
 if h!=ref['sha256'] or p.stat().st_size!=ref['bytes']:
  errors.append(ref['path']+': size/hash mismatch')
actual={str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()}
expected={r['path'] for r in idx['files']}|{'delivery-index.json'}
if actual!=expected: errors.append('file path set mismatch')
for ref in idx['copiedOriginals']:
 p=root/ref['packagePath']
 if hashlib.sha256(p.read_bytes()).hexdigest()!=ref['sha256']:
  errors.append(ref['packagePath']+': original/copy mismatch')
print(json.dumps({'accepted':not errors,'filesChecked':len(idx['files']),
 'byteExactOriginalCopiesChecked':len(idx['copiedOriginals']),
 'cargoOrProjectTestInvoked':False,'errors':errors},indent=2))
raise SystemExit(1 if errors else 0)
'''
(OUT / 'verify-delivery.py').write_text(verify)
# Include the exact preparation program as audit provenance; it is not an acceptance runner.
shutil.copyfile(SCRIPT, OUT / 'prepare-native-evidence-delivery-v1.py')
files=[]
for p in sorted(OUT.rglob('*')):
    if p.is_file():
        files.append({'path':str(p.relative_to(OUT)), 'sha256':sha(p), 'bytes':p.stat().st_size})
index = {'schemaVersion':1, 'createdAtUtc':now(), 'startedAtUtc':started,
         'scope':'Historical phase2/3/4 native evidence delivery; no new test or compile',
         'originalIndicesByteUnmodified':True, 'originalReferencesVerified':len(verified_originals),
         'copiedOriginals':source_refs, 'omittedOriginalBodies':omitted,
         'originalIndices':[{'path':str(BASE/ph/f'{ph}-evidence-index.json'),
                             'sha256':sha(BASE/ph/f'{ph}-evidence-index.json')} for ph in indices],
         'files':files, 'fileCountWithoutThisSelfIndex':len(files),
         'rawTotalBytesWithoutThisSelfIndex':sum(f['bytes'] for f in files),
         'binaryDatabaseOrSourceArchiveBodyIncluded':False,
         'repositoryWrites':False, 'newCargoOrNativeExecution':False}
write(OUT/'delivery-index.json',index)
archive = OUT.with_suffix('.tar.gz')
with tarfile.open(archive,'w:gz',compresslevel=9) as tf:
    tf.add(OUT,arcname=OUT.name,recursive=True)
receipt = {'finishedAtUtc':now(), 'directory':str(OUT), 'deliveryIndex':str(OUT/'delivery-index.json'),
           'deliveryIndexSha256':sha(OUT/'delivery-index.json'), 'archive':str(archive),
           'archiveSha256':sha(archive), 'archiveBytes':archive.stat().st_size,
           'filesIncludingIndex':len(files)+1,
           'originalReferencesVerified':len(verified_originals),
           'protectedArtifactsAndLatestAssetsPathsVerified':len(protected_observed),
           'copiedOriginalFiles':len(source_refs), 'omittedLargeSourceArchiveBodies':len(omitted),
           'sourceOrEvidenceModified':False,'newCargoOrNativeExecution':False}
write(BASE/'native-evidence-delivery-phase2-4-v1-receipt.json',receipt)
print(json.dumps(receipt,indent=2))
