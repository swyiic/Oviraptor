"""Portable resource-pack integrity/shape regressions; temp files only."""
from __future__ import annotations

import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from verify_bundled_fingerprint_pack import VerificationError, compact, main, verify


def rule(identifier='example', name='Example'):
    return {'id': identifier, 'name': name, 'bodyRegex': ['BrandMarker']}


def pack(rules=None, *, schema=1, hash_rules=None):
    rules = rules if rules is not None else [rule()]
    result = {
        'schemaVersion': schema,
        'kind': 'oviraptor-fingerprints',
        'manifest': {'name': 'Synthetic pack', 'version': '1'},
        'fingerprints': rules,
        'sha256': hashlib.sha256(compact(hash_rules if hash_rules is not None else rules).encode()).hexdigest(),
    }
    if schema == 2:
        result['booleanRuntime'] = 'condition-tree-v1'
    return result


class ResourcePackTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='ovi-bundled-pack-test-')
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.directory = self.base / 'resources'
        self.directory.mkdir()

    def write(self, document=None, filename='default.fingerprints.json'):
        document = document if document is not None else pack()
        path = self.directory / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        raw = (json.dumps(document, ensure_ascii=False, indent=2) + '\n').encode()
        path.write_bytes(raw)
        return {'file': filename, 'sha256': hashlib.sha256(raw).hexdigest(),
                'count': len(document['fingerprints']), 'ruleSha256': document['sha256']}

    def manifest(self, entries):
        (self.directory / 'manifest.json').write_text(json.dumps({'schemaVersion': 1, 'packs': entries}))

    def assert_invalid(self, document, message):
        self.manifest([self.write(document)])
        with self.assertRaisesRegex(VerificationError, message):
            verify(self.directory)

    def test_valid_resource_index_and_small_cli_output(self):
        self.manifest([self.write()])
        result = verify(self.directory)
        self.assertTrue(result['valid'])
        self.assertEqual(result['uniqueFingerprints'], 1)
        self.assertEqual(result['networkRequests'], 0)
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            status = main(['--directory', str(self.directory)])
        self.assertEqual(status, 0)
        self.assertTrue(json.loads(stdout.getvalue())['valid'])

    def test_tampering_whole_file_fails_even_for_unchanged_json(self):
        entry = self.write()
        self.manifest([entry])
        with (self.directory / entry['file']).open('a') as stream:
            stream.write(' ')
        with self.assertRaisesRegex(VerificationError, 'whole-file'):
            verify(self.directory)

    def test_manifest_count_and_rule_checksum_are_bound(self):
        entry = self.write()
        self.manifest([{**entry, 'count': 2}])
        with self.assertRaisesRegex(VerificationError, 'count mismatch'):
            verify(self.directory)
        self.manifest([{**entry, 'ruleSha256': 'f' * 64}])
        with self.assertRaisesRegex(VerificationError, 'indexed rule-content'):
            verify(self.directory)

    def test_rule_content_digest_is_independently_recomputed(self):
        document = pack()
        document['sha256'] = 'f' * 64
        self.assert_invalid(document, 'canonical rule-content')

    def test_unknown_vulnerability_and_request_fields_are_rejected(self):
        for field in ['cve', 'CveID', 'poc', 'request', 'ScanSteps', 'detector']:
            with self.subTest(field=field):
                document = pack()
                document['fingerprints'][0][field] = 'untrusted historical content'
                self.assert_invalid(document, 'unknown or missing')

    def test_duplicate_rule_ids_inside_and_across_packs_are_rejected(self):
        self.assert_invalid(pack([rule(), rule()]), 'duplicate fingerprint ID inside')
        first = self.write()
        second = self.write(filename='second.json')
        self.manifest([first, second])
        with self.assertRaisesRegex(VerificationError, 'duplicate fingerprint ID across'):
            verify(self.directory)

    def test_unknown_schema_and_unmarked_tree_are_rejected(self):
        document = pack()
        document['schemaVersion'] = 3
        self.assert_invalid(document, 'schemaVersion')
        document = pack(schema=2)
        del document['booleanRuntime']
        self.assert_invalid(document, 'requires condition-tree')
        document = pack([{'id': 'tree', 'name': 'Tree', 'condition': {'type': 'body', 'pattern': 'Brand'}}])
        self.assert_invalid(document, 'requires schemaVersion 2')

    def test_tree_validation_preserves_nested_groups_and_strict_leaves(self):
        tree = {'id': 'tree', 'name': 'Tree', 'condition': {'type': 'all', 'children': [
            {'type': 'body', 'pattern': 'BrandA'}, {'type': 'body', 'pattern': 'BrandB'}]}}
        self.manifest([self.write(pack([tree], schema=2))])
        self.assertTrue(verify(self.directory)['valid'])
        invalid = copy.deepcopy(tree)
        invalid['condition']['children'] = []
        self.assert_invalid(pack([invalid], schema=2), 'must not be empty')
        invalid['condition'] = {'type': 'body', 'pattern': 'Brand', 'CveID': 'not identification'}
        self.assert_invalid(pack([invalid], schema=2), 'unknown or missing')
        invalid['condition'] = {'type': 'not', 'pattern': 'Brand'}
        self.assert_invalid(pack([invalid], schema=2), 'unknown condition')

    def test_tree_depth_nodes_mixed_signals_and_flat_all_are_rejected(self):
        node = {'type': 'body', 'pattern': 'Brand'}
        for _ in range(16):
            node = {'type': 'all', 'children': [node]}
        self.assert_invalid(pack([{'id': 'tree', 'name': 'Tree', 'condition': node}], schema=2), 'depth or node')
        node = {'type': 'all', 'children': [{'type': 'body', 'pattern': 'Brand'}] * 256}
        self.assert_invalid(pack([{'id': 'tree', 'name': 'Tree', 'condition': node}], schema=2), 'depth or node')
        self.assert_invalid(pack([{**rule(), 'condition': {'type': 'body', 'pattern': 'Brand'}}], schema=2), 'mixed with legacy')
        self.assert_invalid(pack([{**rule(), 'matchMode': 'all'}]), 'flat matchMode')

    def test_unsafe_path_status_and_control_characters_are_rejected(self):
        for path in ['https://example.com/', '//other/', '/../secret', '/%2e%2e/file', '/x?token=example']:
            with self.subTest(path=path):
                self.assert_invalid(pack([{'id': 'path', 'name': 'Path', 'pathSignals': [
                    {'path': path, 'status': [200], 'bodyRegex': 'Brand'}]}]), 'unsafe characteristic path')
        self.assert_invalid(pack([{'id': 'path', 'name': 'Path', 'pathSignals': [
            {'path': '/login', 'status': [True], 'bodyRegex': 'Brand'}]}]), 'invalid integer')
        self.assert_invalid(pack([{**rule(), 'bodyRegex': ['Brand\x00']}]), 'control characters')

    def test_canonicalization_matches_documented_rust_field_order(self):
        original = {'id': 'example', 'name': '示例', 'bodyRegex': ['BrandB', 'BrandA', 'BrandA'],
                    'headerRegex': {'X-Powered-By': 'Runtime', 'Server': 'Vendor'}, 'matchMode': 'any'}
        canonical = {'id': 'example', 'name': '示例', 'headerRegex': {'server': 'Vendor', 'x-powered-by': 'Runtime'},
                     'bodyRegex': ['BrandA', 'BrandB']}
        self.manifest([self.write(pack([original], hash_rules=[canonical]))])
        self.assertTrue(verify(self.directory)['valid'])

    def test_resource_paths_refuse_traversal_and_symlinks(self):
        entry = self.write()
        for filename in ['../outside.json', '/absolute.json', 'folder/../default.fingerprints.json', './default.fingerprints.json', 'folder\\file.json']:
            with self.subTest(filename=filename):
                self.manifest([{**entry, 'file': filename}])
                with self.assertRaisesRegex(VerificationError, 'path'):
                    verify(self.directory)
        other = self.base / 'external.json'
        other.write_bytes((self.directory / entry['file']).read_bytes())
        linked = self.directory / 'linked.json'
        linked.symlink_to(other)
        self.manifest([{**entry, 'file': 'linked.json'}])
        with self.assertRaisesRegex(VerificationError, 'symbolic'):
            verify(self.directory)
        subdirectory = self.base / 'external-directory'
        subdirectory.mkdir()
        (subdirectory / 'pack.json').write_bytes(other.read_bytes())
        (self.directory / 'linked-directory').symlink_to(subdirectory, target_is_directory=True)
        self.manifest([{**entry, 'file': 'linked-directory/pack.json'}])
        with self.assertRaisesRegex(VerificationError, 'symbolic'):
            verify(self.directory)

    def test_duplicate_json_keys_and_duplicate_indexed_files_are_rejected(self):
        self.manifest([self.write()])
        (self.directory / 'manifest.json').write_text('{"schemaVersion":1,"schemaVersion":1,"packs":[]}')
        with self.assertRaisesRegex(VerificationError, 'duplicate object keys'):
            verify(self.directory)
        entry = self.write()
        self.manifest([entry, entry])
        with self.assertRaisesRegex(VerificationError, 'duplicate indexed file'):
            verify(self.directory)

    def test_cli_failure_is_structured_and_read_only(self):
        entry = self.write()
        self.manifest([{**entry, 'count': False}])
        before = {p.name: p.read_bytes() for p in self.directory.iterdir()}
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            status = main(['--directory', str(self.directory)])
        self.assertEqual(status, 1)
        self.assertFalse(json.loads(stderr.getvalue())['valid'])
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.directory.iterdir()})


if __name__ == '__main__':
    unittest.main()
