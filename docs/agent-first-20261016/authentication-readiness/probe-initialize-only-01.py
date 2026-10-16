#!/usr/bin/env python3
"""Exactly one ACP initialize inside the pinned scratch image. No auth/session/prompt."""
from pathlib import Path
import datetime, hashlib, json, os, selectors, subprocess, time

ROOT = Path('/workspace/validation/agent-first-20261016/grok-cli/isolation-review')
OUT = ROOT / 'initialize-only-01'
CONFIG = ROOT / 'initialize-docker-client-config-01'
DOCKER = '/usr/local/bin/docker'
IMAGE = 'sha256:3527c4cd7e66a5b1911957bc24586884c84f214787d0f4b7ead7025b0ee70c1d'
EXPECTED_BINARY = '10e6af64b24724fc1074c373e906b9ad121110044dddcbd97ad43c3a25f96d5f'
ENV = {'PATH': '/usr/local/bin:/usr/bin:/bin', 'LANG': 'C.UTF-8'}
MAX_STDOUT = 2 * 1024 * 1024
MAX_STDERR = 512 * 1024
MAX_LINE = 1024 * 1024
ledger = []

def now(): return datetime.datetime.now(datetime.timezone.utc).isoformat()
def ref(path):
    data = path.read_bytes()
    return {'path': str(path), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
def run(label, argv):
    start = now(); result = subprocess.run(argv, env=ENV, capture_output=True, timeout=20)
    stdout, stderr = OUT / (label + '.stdout'), OUT / (label + '.stderr')
    stdout.write_bytes(result.stdout); stderr.write_bytes(result.stderr)
    record = {'label': label, 'argv': argv, 'startedAt': start, 'finishedAt': now(),
              'actualProcessExitCode': result.returncode, 'stdout': ref(stdout), 'stderr': ref(stderr)}
    ledger.append(record); (OUT / 'command-ledger.json').write_text(json.dumps(ledger, indent=2) + '\n')
    if result.returncode: raise RuntimeError(f'{label} actual exit {result.returncode}; preserved raw evidence')
    return result.stdout
def isolation(item, *, created=False):
    hc = item['HostConfig']
    assert item['Image'] == IMAGE and item['Config']['User'] == '10001:10001'
    assert item['Path'] == '/bin/grok' and item['Args'] == ['--no-auto-update', 'agent', '--no-leader', 'stdio']
    assert item['Config']['Env'] == image['Config']['Env']
    assert hc['NetworkMode'] == 'none' and hc['IpcMode'] == 'none' and hc['ReadonlyRootfs'] and not hc['Privileged']
    assert hc['CapDrop'] == ['ALL'] and 'no-new-privileges' in hc['SecurityOpt']
    assert hc['Memory'] == 1073741824 and hc['PidsLimit'] == 128 and hc['NanoCpus'] == 1000000000
    assert not hc.get('Binds') and not hc.get('VolumesFrom') and not hc.get('Devices')
    assert set(hc['Tmpfs']) == {'/home/grok', '/work', '/tmp'}
    assert all(mount['Type'] == 'tmpfs' for mount in item.get('Mounts', []))
    assert item['Config']['OpenStdin'] and not item['Config']['Tty']
    if created: assert item['State']['Status'] == 'created'

assert not OUT.exists() and not CONFIG.exists(), 'Never overwrite prior attempts or reuse client configuration'
OUT.mkdir(); CONFIG.mkdir()
binary = ROOT.parent / 'official-alternatives/runtime/grok-1.0.60'
assert ref(binary)['sha256'] == EXPECTED_BINARY
prior_receipt = ROOT / 'scratch-version-help-actual-receipt-01.json'
assert ref(prior_receipt)['sha256'] == '6319b2ecf3a3df6d6df08e2c678fd4de5cfef01c559d4dee9bbf77e27fa7263a'
old_image = json.loads((ROOT / 'scratch-image-inspect-01.stdout').read_text())[0]
image = json.loads(run('image-inspect-before', [DOCKER, '--config', str(CONFIG), 'image', 'inspect', IMAGE]))[0]
assert image['Id'] == IMAGE and image['Config'] == old_image['Config']
old_ledger = json.loads((ROOT / 'version-help-command-ledger-01.json').read_text())
template = next(entry['argv'] for entry in old_ledger if entry['label'] == 'isolated-help-3-01')
argv = template[:template.index(IMAGE) + 1]
argv[argv.index('--config') + 1] = str(CONFIG)
argv[argv.index('run')] = 'create'
argv[argv.index('--name') + 1] = 'oviraptor-grok-initialize-only-01'
argv[argv.index('--cidfile') + 1] = str(OUT / 'container-id.txt')
argv += ['--no-auto-update', 'agent', '--no-leader', 'stdio']
cid = run('container-create', argv).decode().strip()
assert len(cid) == 64 and all(c in '0123456789abcdef' for c in cid)
created = json.loads(run('container-inspect-created', [DOCKER, '--config', str(CONFIG), 'inspect', cid]))[0]
isolation(created, created=True)

request = {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': 1,
    'clientInfo': {'name': 'Oviraptor-isolated-initialize-probe', 'version': '0.1.0'},
    'clientCapabilities': {'fs': {'readTextFile': False, 'writeTextFile': False}, 'terminal': False}}}
wire = (json.dumps(request, separators=(',', ':'), ensure_ascii=False) + '\n').encode()
assert wire.count(b'\n') == 1 and len(wire) < 1024
(OUT / 'only-sent-frame.ndjson').write_bytes(wire)
start = now(); clock = time.monotonic()
start_argv = [DOCKER, '--config', str(CONFIG), 'start', '-a', '-i', cid]
proc = subprocess.Popen(start_argv, env=ENV, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
selector = selectors.DefaultSelector()
selector.register(proc.stdout, selectors.EVENT_READ, 'stdout'); selector.register(proc.stderr, selectors.EVENT_READ, 'stderr')
stdout, stderr, pending = bytearray(), bytearray(), bytearray()
response = None; frames = []; failure = None; eof_sent = False; shutdown_deadline = None
try:
    proc.stdin.write(wire); proc.stdin.flush()
except (BrokenPipeError, OSError) as error:
    failure = 'stdin_transport_failure'
deadline = clock + 35
while selector.get_map():
    current = time.monotonic()
    if not eof_sent and (response is not None or failure or current >= deadline):
        if response is None and not failure: failure = 'initialize_timeout'
        try: proc.stdin.close()
        except (BrokenPipeError, OSError): pass
        eof_sent = True; shutdown_deadline = current + 15
    if eof_sent and current >= shutdown_deadline:
        if proc.poll() is None:
            if not failure: failure = 'eof_shutdown_timeout'
            run('container-stop-no-accepted-work', [DOCKER, '--config', str(CONFIG), 'stop', '--time', '3', cid])
        break
    for key, _ in selector.select(0.25):
        chunk = os.read(key.fileobj.fileno(), 65536)
        if not chunk:
            selector.unregister(key.fileobj); continue
        if key.data == 'stderr':
            remaining = MAX_STDERR - len(stderr); stderr.extend(chunk[:remaining])
            if len(chunk) > remaining: failure = failure or 'stderr_budget_exceeded'
            continue
        remaining = MAX_STDOUT - len(stdout); stdout.extend(chunk[:remaining])
        if len(chunk) > remaining: failure = failure or 'stdout_budget_exceeded'; continue
        pending.extend(chunk)
        if len(pending) > MAX_LINE: failure = failure or 'ndjson_frame_budget_exceeded'; pending.clear(); continue
        while b'\n' in pending:
            line, _, remainder = pending.partition(b'\n'); pending = bytearray(remainder)
            try:
                value = json.loads(line.decode('utf-8'), parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
                if not isinstance(value, dict) or value.get('jsonrpc') != '2.0': raise ValueError('not_jsonrpc')
                frames.append(value)
                if value.get('id') == 1 and 'method' not in value:
                    if response is not None or ('result' in value) == ('error' in value): raise ValueError('duplicate_or_invalid_response')
                    response = value
                elif 'id' in value:
                    failure = failure or 'unexpected_server_request_or_response'
            except (ValueError, UnicodeDecodeError): failure = failure or 'invalid_ndjson_jsonrpc'
    if proc.poll() is not None and not selector.get_map(): break

if not eof_sent:
    try: proc.stdin.close()
    except (BrokenPipeError, OSError): pass
    eof_sent = True
if pending: failure = failure or 'partial_ndjson_at_eof'
if response is None: failure = failure or 'no_initialize_response'
try: exit_code = proc.wait(timeout=5)
except subprocess.TimeoutExpired:
    failure = failure or 'attach_exit_timeout'
    run('container-stop-final-no-accepted-work', [DOCKER, '--config', str(CONFIG), 'stop', '--time', '3', cid])
    exit_code = proc.wait(timeout=10)
selector.close()
(OUT / 'actual-stdout.ndjson').write_bytes(stdout); (OUT / 'actual-stderr.log').write_bytes(stderr)
(OUT / 'received-frames.json').write_text(json.dumps(frames, ensure_ascii=False, indent=2) + '\n')
if response is not None: (OUT / 'initialize-response.json').write_text(json.dumps(response, ensure_ascii=False, indent=2) + '\n')
ledger.append({'label': 'actual-initialize-only-stdio', 'argv': start_argv, 'startedAt': start, 'finishedAt': now(),
    'actualProcessExitCode': exit_code, 'durationSeconds': round(time.monotonic() - clock, 6),
    'onlyWrittenFrame': ref(OUT / 'only-sent-frame.ndjson'), 'stdout': ref(OUT / 'actual-stdout.ndjson'), 'stderr': ref(OUT / 'actual-stderr.log')})
(OUT / 'command-ledger.json').write_text(json.dumps(ledger, indent=2) + '\n')
finished = json.loads(run('container-inspect-finished', [DOCKER, '--config', str(CONFIG), 'inspect', cid]))[0]
isolation(finished); assert not finished['State']['Running']
run('container-remove', [DOCKER, '--config', str(CONFIG), 'rm', cid])
assert not list(CONFIG.iterdir()) and ref(binary)['sha256'] == EXPECTED_BINARY
result = response.get('result') if response is not None else None
negotiated = isinstance(result, dict) and result.get('protocolVersion') == 1
auth_methods = result.get('authMethods') if isinstance(result, dict) else None
adapter_cached_method = isinstance(auth_methods, list) and any(isinstance(item, dict) and item.get('id') == 'cached_token' for item in auth_methods)
record = {'schemaVersion': 1, 'finishedAt': now(), 'scope': 'Exactly one initialize; no authenticate, session/new, prompt, model request, or business work accepted.',
    'imageId': IMAGE, 'binary': ref(binary), 'priorVersionHelpReceipt': ref(prior_receipt), 'driver': ref(Path(__file__)),
    'commandLedger': ref(OUT / 'command-ledger.json'), 'onlySentFrame': ref(OUT / 'only-sent-frame.ndjson'),
    'stdout': ref(OUT / 'actual-stdout.ndjson'), 'stderr': ref(OUT / 'actual-stderr.log'), 'frames': ref(OUT / 'received-frames.json'),
    'actualDockerAttachExitCode': exit_code, 'actualContainerExitCode': finished['State']['ExitCode'],
    'failureCode': failure, 'negotiatedProtocol1': negotiated, 'responseIsError': bool(response and 'error' in response),
    'advertisedAuthMethods': auth_methods, 'adapterWouldFindCachedTokenMethod': adapter_cached_method,
    'authenticated': False, 'authenticationReadiness': 'NOT_AUTHENTICATED; advertised authMethods do not prove a cached credential is present or accepted.',
    'normalEofSent': eof_sent, 'normalEofExitedWithoutForcedStop': not any('stop' in item['label'] for item in ledger),
    'isolationInspectChecksPassed': True, 'hostDirectoryMounts': [], 'hostHomeReadOrModified': False,
    'sourceBuildMatchesNpmExact': False, 'paidOrRealModelAcceptance': False}
if response is not None: record['actualInitializeResponse'] = ref(OUT / 'initialize-response.json')
path = OUT / 'actual-initialize-only-receipt.json';path.write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(ref(path)))
if failure or not negotiated or exit_code != 0 or finished['State']['ExitCode'] != 0: raise SystemExit(1)
