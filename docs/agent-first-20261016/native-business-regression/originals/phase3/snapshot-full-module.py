import sys,json,hashlib,subprocess,datetime,stat,re
from pathlib import Path
ROOT=Path('/workspace/Oviraptor-agent-first-20261016');OUT=Path(__file__).resolve().parent
POLICY=json.loads(Path('/workspace/validation/rust-accepted-full-input-manifest.json').read_text())['audit_policy']

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def inventory(paths):
 found={}
 def visit(p):
  st=p.lstat()
  if stat.S_ISLNK(st.st_mode):raise RuntimeError('Symlink input is not accepted: '+str(p))
  if stat.S_ISDIR(st.st_mode):
   for c in sorted(p.iterdir()):visit(c)
  elif stat.S_ISREG(st.st_mode):found[p.relative_to(ROOT).as_posix()]=sha(p)
  else:raise RuntimeError('Unsupported input: '+str(p))
 for p in paths:visit(p)
 return dict(sorted(found.items()))
def snapshot(binary=None):
 helper=ROOT/'src-tauri/src/commands/agent_tests_backend_literal.rs';source=helper.read_text()
 m=re.search(r'const RETIREMENT_SOURCE_SCOPES:.*?= &\[(.*?)\];',source,re.S);assert m
 assert re.findall(r'"([^"]+)"',m[1])==POLICY['source_roots'],'Audit scope policy drift needs review'
 roots=[];missing=[]
 for name in POLICY['source_roots']:
  p=ROOT/name
  if not p.exists() and not p.is_symlink() and name in POLICY['optional_only_if_absent']:missing.append(name)
  else:roots.append(p)
 for name in POLICY['extra_roots']:
  p=ROOT/name
  if p.exists() or p.is_symlink():roots.append(p)
 ext=set(POLICY['root_build_inputs']['extensions']);names=set(POLICY['root_build_inputs']['names'])
 for name in POLICY['root_build_inputs']['directories']:
  for p in sorted((ROOT/name).iterdir()):
   if not p.is_dir() and (p.suffix.lstrip('.') in ext or p.name in names):roots.append(p)
 native=[ROOT/'src-tauri',ROOT/'packages/native-app-sdk',ROOT/'tools/run_with_subreaper.py']
 if (ROOT/'.cargo').exists():native.append(ROOT/'.cargo')
 tracked=subprocess.check_output(['git','ls-files','-z'],cwd=ROOT).decode().rstrip('\0').split('\0')
 r={'atUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'sourceHead':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'workingTreeStatus':subprocess.check_output(['git','status','--porcelain=v1','--untracked-files=all'],cwd=ROOT,text=True).splitlines(),'nativeInputs':inventory(native),'auditInputs':inventory(roots),'trackedInputs':inventory([ROOT/p for p in tracked]),'auditPolicy':dict(POLICY,missing_optional_roots=missing),'auditPolicySourceSha256':sha(helper)}
 if binary:r['binary']={'path':str(binary),'sha256':sha(binary)}
 return r
