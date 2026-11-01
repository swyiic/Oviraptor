#!/usr/bin/env python3
"""Offline regression: production rule-pack Git/SQLite sync with temporary data.

Compiles the current production helper bodies against cached Rust dependencies.
The adapter replaces only App DB open and unrelated UI text scrubbing; no real
App database, rule-pack cache, network service or application startup is used.
"""
from pathlib import Path
import json
import os
import sqlite3
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'src-tauri/src/commands/rule_packs.rs'


def build_adapter(directory):
    deps = ROOT / 'src-tauri/target/debug/deps'
    names = ('rusqlite', 'serde_json')
    libraries = {name: sorted(deps.glob(f'lib{name}-*.rlib'), key=lambda p: p.stat().st_mtime_ns, reverse=True) for name in names}
    if not all(libraries.values()):
        deps = ROOT / 'src-tauri/target/release/deps'
        libraries = {name: sorted(deps.glob(f'lib{name}-*.rlib'), key=lambda p: p.stat().st_mtime_ns, reverse=True) for name in names}
    if not all(libraries.values()):
        raise RuntimeError('Build the project offline once to supply cached rusqlite/serde_json dependencies')
    source = SOURCE.read_text()
    source = source[source.index('fn rule_pack_root('):source.index('#[tauri::command]\npub fn sync_security_rule_pack(')]
    adapter = r'''
#![allow(dead_code)]
use std::{fs,io::{BufRead,BufReader},path::{Path,PathBuf},process::{Command,Stdio}};
use rusqlite::params;
mod db { pub fn open(path:&std::path::Path)->Result<rusqlite::Connection,String>{rusqlite::Connection::open(path).map_err(|e|e.to_string())} }
mod agent_runtime { pub mod secrets { pub fn redact_text_with(text:&str,_:Option<()>)->String{text.into()} } }
'''+source+r'''
fn main(){
 let args=std::env::args().collect::<Vec<_>>();
 if args.get(1).map(String::as_str)==Some("effective-source") {
  assert_eq!(args.len(),5);println!("{}",serde_json::to_string(&effective_rule_pack_source(&args[2],&args[3],&args[4])).unwrap());return;
 }
 assert_eq!(args.len(),8);
 let (repo,reference)=effective_rule_pack_source(&args[4],&args[5],&args[6]);
 run_rule_pack_sync(Path::new(&args[1]),args[2].parse().unwrap(),Path::new(&args[3]),&args[4],&repo,&reference,&args[7]);
}
'''
    path = directory / 'rule_pack_sync_adapter.rs'
    path.write_text(adapter)
    binary = directory / 'rule-pack-sync-adapter'
    command = ['rustc', '--edition=2021', str(path), '-L', f'dependency={deps}', '-o', str(binary)]
    for name in names:
        command += ['--extern', f'{name}={libraries[name][0]}']
    subprocess.run(command, check=True, capture_output=True, text=True)
    return binary


class RulePackSyncTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.build_dir = tempfile.TemporaryDirectory(prefix='ovi-rule-sync-build-')
        cls.binary = build_adapter(Path(cls.build_dir.name))

    @classmethod
    def tearDownClass(cls):
        cls.build_dir.cleanup()

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='ovi-rule-sync-case-')
        self.root = Path(self.temporary.name)
        self.repository = self.root / 'upstream'
        self.repository.mkdir()
        # All subprocess configuration is isolated from the user's Git config.
        self.env = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
                    'GIT_AUTHOR_NAME': 'Fixture', 'GIT_AUTHOR_EMAIL': 'fixture@example.invalid',
                    'GIT_COMMITTER_NAME': 'Fixture', 'GIT_COMMITTER_EMAIL': 'fixture@example.invalid',
                    'GIT_TERMINAL_PROMPT': '0'}
        self.git('init', '-b', 'main')
        (self.repository / 'keep.yaml').write_text('version: 1\n')
        (self.repository / 'removed.yaml').write_text('remove me\n')
        self.first = self.commit('initial')
        self.remote = self.repository.as_uri()
        self.checkout = self.root / 'cache'
        self.database = self.root / 'test.sqlite'
        schema = (ROOT / 'src-tauri/src/db_initialize_columns.rs').read_text()
        schema = schema[schema.index('CREATE TABLE IF NOT EXISTS security_rule_packs ('):]
        schema = schema[:schema.index(');') + 2]
        with sqlite3.connect(self.database) as conn:
            conn.execute(schema)
            conn.execute("INSERT INTO security_rule_packs(key,name,engine,repository,reference,status) VALUES('fixture','Fixture','semgrep',?,'main','syncing')", [self.remote])

    def tearDown(self):
        self.temporary.cleanup()

    def git(self, *args, cwd=None):
        return subprocess.run(['git', *args], cwd=cwd or self.repository, env=self.env, check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, message):
        self.git('add', '-A')
        self.git('commit', '-m', message)
        return self.git('rev-parse', 'HEAD')

    def next_commit(self):
        (self.repository / 'keep.yaml').write_text('version: 2\n')
        (self.repository / 'removed.yaml').unlink()
        (self.repository / 'added.yaml').write_text('new rule\n')
        return self.commit('change')

    def row(self):
        with sqlite3.connect(self.database) as conn:
            conn.row_factory = sqlite3.Row
            return dict(conn.execute('SELECT * FROM security_rule_packs WHERE id=1').fetchone())

    def sync(self, old='', repo=None, reference='main', key='fixture'):
        with sqlite3.connect(self.database) as conn:
            conn.execute("UPDATE security_rule_packs SET status='syncing',error='',version=? WHERE id=1", [old])
        subprocess.run([str(self.binary), str(self.database), '1', str(self.checkout), key, repo or self.remote, reference, old], env=self.env, check=True, capture_output=True, text=True)
        return self.row()

    def test_owasp_source_migration_is_limited_to_exact_builtin_defaults(self):
        old = 'https://github.com/OWASP/Benchmark.git'
        new = 'https://github.com/OWASP-Benchmark/BenchmarkJava.git'
        cases = [
            ('owasp-benchmark', old, 'master', [new, 'master']),
            ('owasp-benchmark', old, '', [new, 'master']),
            ('owasp-benchmark', new, 'master', [new, 'master']),
            ('owasp-benchmark', old, 'main', [old, 'main']),
            ('owasp-benchmark', old, 'release/v1', [old, 'release/v1']),
            ('owasp-benchmark', 'https://github.com/example/Benchmark.git', 'master', ['https://github.com/example/Benchmark.git', 'master']),
            ('custom-benchmark', old, 'master', [old, 'master']),
        ]
        for key, repo, reference, expected in cases:
            with self.subTest(key=key, repo=repo, reference=reference):
                result = subprocess.run([str(self.binary), 'effective-source', key, repo, reference], check=True, capture_output=True, text=True)
                self.assertEqual(json.loads(result.stdout), expected)

    def test_known_owasp_origin_migrates_and_failed_fetch_can_retry(self):
        self.sync()
        old = 'https://github.com/OWASP/Benchmark.git'
        new = 'https://github.com/OWASP-Benchmark/BenchmarkJava.git'
        self.git('remote', 'set-url', 'origin', old, cwd=self.checkout)
        # Redirect the canonical test URL to our local repository, never GitHub.
        self.env.update({'GIT_CONFIG_COUNT': '1', 'GIT_CONFIG_KEY_0': f'url.{self.remote}.insteadOf', 'GIT_CONFIG_VALUE_0': new})
        row = self.sync(self.first, repo=old, reference='master', key='owasp-benchmark')
        self.assertEqual(row['status'], 'error')  # fixture master does not exist yet
        self.assertEqual(self.git('config', '--get', 'remote.origin.url', cwd=self.checkout), new)
        self.assertEqual(row['version'], self.first)
        self.git('branch', 'master')
        row = self.sync(self.first, repo=old, reference='master', key='owasp-benchmark')
        self.assertEqual((row['status'], row['repository'], row['reference']), ('ready', new, 'master'))
        self.assertEqual(row['version'], self.first)

    def test_known_origin_pair_does_not_rewrite_custom_pack_or_branch(self):
        self.sync()
        old = 'https://github.com/OWASP/Benchmark.git'
        new = 'https://github.com/OWASP-Benchmark/BenchmarkJava.git'
        self.git('remote', 'set-url', 'origin', old, cwd=self.checkout)
        for key, reference in [('custom-benchmark', 'master'), ('owasp-benchmark', 'other-branch')]:
            row = self.sync(self.first, repo=new, reference=reference, key=key)
            self.assertEqual(row['status'], 'error')
            self.assertIn('远端与配置的仓库不一致', row['error'])
            self.assertEqual(self.git('config', '--get', 'remote.origin.url', cwd=self.checkout), old)

    def test_first_sync_records_full_id_and_initial_file_list(self):
        row = self.sync()
        self.assertEqual((row['status'], row['progress_stage'], row['version']), ('ready', 'complete', self.first))
        self.assertEqual((row['added_count'], row['modified_count'], row['deleted_count']), (2, 0, 0))
        self.assertEqual(row['previous_version'], '')
        self.assertEqual(len(json.loads(row['change_summary'])), 2)

    def test_short_id_is_resolved_and_pinned_before_shallow_update(self):
        self.sync()
        second = self.next_commit()
        row = self.sync(self.first[:7])
        self.assertEqual((row['status'], row['previous_version'], row['version']), ('ready', self.first, second))
        self.assertEqual((row['added_count'], row['modified_count'], row['deleted_count']), (1, 1, 1))
        self.assertEqual(self.git('rev-parse', 'refs/oviraptor/rule-pack-baseline', cwd=self.checkout), self.first)
        self.assertTrue((self.checkout / '.git/shallow').exists())

    def test_missing_historical_id_recovers_actual_local_head_explicitly(self):
        self.sync()
        second = self.next_commit()
        row = self.sync('40b8c63')
        self.assertEqual(row['status'], 'ready')
        self.assertEqual(row['progress_stage'], 'complete-local-baseline')
        self.assertEqual((row['previous_version'], row['version']), (self.first, second))
        self.assertEqual((row['added_count'], row['modified_count'], row['deleted_count']), (1, 1, 1))
        self.assertIn('本地版本', row['progress_message'])

    def test_missing_old_id_with_new_cache_reports_unknown_diff_then_recovers(self):
        row = self.sync('c1b19fb1')
        self.assertEqual((row['status'], row['progress_stage'], row['version']), ('ready', 'complete-no-baseline', self.first))
        self.assertEqual(row['previous_version'], 'c1b19fb1')
        self.assertEqual((row['added_count'], row['modified_count'], row['deleted_count']), (0, 0, 0))
        self.assertEqual(json.loads(row['change_summary']), [])
        self.assertIn('不统计增删改', row['progress_message'])
        again = self.sync(row['version'])
        self.assertEqual((again['status'], again['progress_stage'], again['version']), ('ready', 'complete', self.first))
        self.assertEqual((again['added_count'], again['modified_count'], again['deleted_count']), (0, 0, 0))

    def test_same_short_head_and_filename_collision_are_unambiguous(self):
        self.sync()
        (self.repository / self.first[:7]).write_text('filename resembles a commit\n')
        second = self.commit('add ambiguous filename')
        row = self.sync(self.first[:7])
        self.assertEqual((row['status'], row['version'], row['added_count']), ('ready', second, 1))
        row = self.sync(second[:8])
        self.assertEqual((row['status'], row['progress_stage'], row['modified_count']), ('ready', 'complete', 0))

    def test_missing_remote_is_still_error_and_does_not_advance_version(self):
        row = self.sync('40b8c63', repo=(self.root / 'missing-repo').as_uri())
        self.assertEqual((row['status'], row['progress_stage'], row['version']), ('error', 'error', '40b8c63'))
        self.assertTrue(row['error'])

    def test_fetch_failure_keeps_working_checkout_and_version(self):
        self.sync()
        row = self.sync(self.first, reference='missing-branch')
        self.assertEqual((row['status'], row['version']), ('error', self.first))
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.checkout), self.first)
        self.assertTrue(row['error'])

    def test_different_origin_is_not_reset(self):
        self.sync()
        row = self.sync(self.first, repo=(self.root / 'other-repo').as_uri())
        self.assertEqual(row['status'], 'error')
        self.assertIn('远端与配置的仓库不一致', row['error'])
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.checkout), self.first)

    def test_invalid_stored_revision_is_never_interpreted_as_git_expression(self):
        row = self.sync('--output=outside')
        self.assertEqual((row['status'], row['progress_stage']), ('ready', 'complete-no-baseline'))
        self.assertFalse((self.root / 'outside').exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
