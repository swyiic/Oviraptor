import importlib.util
import json
import os
import shutil
import sqlite3
import tempfile
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('retirement',Path(__file__).with_name('retire_nest_data.py'))
r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)

class RetirementContracts(unittest.TestCase):
    def setUp(self):
        self.root=Path(tempfile.mkdtemp(prefix='oviraptor-retirement-contract-'))
        self.path=self.root/'database.sqlite3';self.backup=self.root/'backup.sqlite3'
        c=sqlite3.connect(self.path)
        c.executescript('''CREATE TABLE assets(id INTEGER PRIMARY KEY,value BLOB);
        CREATE TABLE native_rows(id TEXT PRIMARY KEY,payload TEXT);
        CREATE TABLE strix_skills(id INTEGER PRIMARY KEY,name TEXT);
        CREATE TABLE strix_knowledge_entries(id INTEGER PRIMARY KEY,skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,value TEXT);
        CREATE TABLE strix_learning_candidates(id INTEGER PRIMARY KEY,value TEXT);
        CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT);
        CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,settings_json TEXT,updated_at TEXT);
        CREATE TABLE sentinel_checkpoints(scan_id TEXT,url TEXT,stage TEXT,raw_json TEXT,PRIMARY KEY(scan_id,url,stage));
        CREATE TABLE sentinel_findings(id INTEGER PRIMARY KEY,stage TEXT,record_json TEXT);
        INSERT INTO assets VALUES(9,x'0031ff');
        INSERT INTO native_rows VALUES('native','{"native":"keep Strix as source evidence"}');
        INSERT INTO strix_skills VALUES(1,'old');
        INSERT INTO strix_knowledge_entries VALUES(1,1,'old knowledge');
        INSERT INTO strix_learning_candidates VALUES(1,'old candidate');
        INSERT INTO app_settings VALUES('strix-current-attempt:task','old');
        INSERT INTO app_settings VALUES('native-key','keep');
        INSERT INTO sentinel_checkpoints VALUES('task','url','strix_run:old','old');
        INSERT INTO sentinel_checkpoints VALUES('task','url','native_agent_state','{"current":true}');
        INSERT INTO sentinel_findings VALUES(1,'strix','old');
        INSERT INTO sentinel_findings VALUES(2,'native-agent','{"current":true}');''')
        c.execute('INSERT INTO config_profiles VALUES(1,?,?)',(json.dumps({'strixApiKey':'old','modelApiKey':'native-secret','custom':{'strixExample':'user'},'modelProfiles':None}), 'original'))
        c.commit();c.close()
        self.refresh()
    def refresh(self):
        shutil.copyfile(self.path,self.backup)
        self.plan=r.inventory(self.path,self.backup)
    def snap(self):
        c=r.connect(self.path);s=r.snapshot(c);c.close();return s
    def tearDown(self):
        shutil.rmtree(self.root)
    def test_exact_cleanup_keeps_asset_native_json_and_nested_fields(self):
        result=r.apply(self.plan);self.assertEqual(result['changed_rows'],7)
        c=r.connect(self.path)
        raw=c.execute('SELECT settings_json,updated_at FROM config_profiles').fetchone()
        self.assertEqual(json.loads(raw[0]),{'modelApiKey':'native-secret','custom':{'strixExample':'user'},'modelProfiles':None})
        self.assertEqual(raw[1],'original')
        self.assertEqual(c.execute('SELECT value FROM assets').fetchone()[0],bytes.fromhex('0031ff'))
        self.assertEqual(c.execute('SELECT stage FROM sentinel_findings').fetchall(),[('native-agent',)])
        self.assertEqual(c.execute('SELECT stage FROM sentinel_checkpoints').fetchall(),[('native_agent_state',)])
        c.close()
    def test_hardlinked_backup_is_refused_before_inventory_or_write(self):
        alias=self.root/'hardlink-backup.sqlite3';os.link(self.path,alias)
        before=self.snap()
        with self.assertRaisesRegex(ValueError,'backup_is_live_database'):r.inventory(self.path,alias)
        plan=dict(self.plan);plan['backup']=str(alias)
        with self.assertRaisesRegex(ValueError,'backup_is_live_database'):r.apply(plan)
        self.assertEqual(self.snap(),before)
    def test_committed_plan_replay_is_readonly_and_verified(self):
        r.apply(self.plan);before=self.snap()
        result=r.apply(self.plan)
        self.assertTrue(result['already_applied']);self.assertEqual(result['changed_rows'],0)
        self.assertEqual(self.snap(),before)
    def test_stale_source_is_refused_without_writes(self):
        c=sqlite3.connect(self.path);c.execute("UPDATE assets SET value='new asset'");c.commit();c.close()
        before=self.snap()
        with self.assertRaisesRegex(ValueError,'source_changed'):r.apply(self.plan)
        self.assertEqual(self.snap(),before)
    def test_tampered_plan_cannot_expand_to_assets(self):
        self.plan['actions'].append({'table':'assets','rowid':9,'kind':'delete','original_hash':'invalid','removed_keys':[]})
        before=self.snap()
        with self.assertRaisesRegex(ValueError,'plan_scope'):r.apply(self.plan)
        self.assertEqual(self.snap(),before)
    def test_backup_changed_cannot_authorize_any_write(self):
        self.backup.write_bytes(b'bad backup');before=self.snap()
        with self.assertRaisesRegex(ValueError,'backup_changed'):r.apply(self.plan)
        self.assertEqual(self.snap(),before)
    def test_collateral_asset_trigger_is_denied_and_rolled_back(self):
        c=sqlite3.connect(self.path);c.execute("CREATE TRIGGER asset_fault AFTER DELETE ON app_settings BEGIN DELETE FROM assets; END");c.commit();c.close();self.refresh()
        before=self.snap()
        with self.assertRaises(sqlite3.DatabaseError):r.apply(self.plan)
        self.assertEqual(self.snap(),before)
    def test_current_checkpoint_trigger_changes_roll_back(self):
        c=sqlite3.connect(self.path);c.execute("CREATE TRIGGER native_fault AFTER DELETE ON app_settings BEGIN UPDATE sentinel_checkpoints SET raw_json='damage' WHERE stage='native_agent_state'; END");c.commit();c.close();self.refresh()
        before=self.snap()
        with self.assertRaisesRegex(ValueError,'final_data_scope'):r.apply(self.plan)
        self.assertEqual(self.snap(),before)
    def test_ignored_delete_never_claims_cleanup(self):
        c=sqlite3.connect(self.path);c.execute("CREATE TRIGGER ignored BEFORE DELETE ON app_settings BEGIN SELECT RAISE(IGNORE); END");c.commit();c.close();self.refresh()
        before=self.snap()
        with self.assertRaisesRegex(ValueError,'write_unconfirmed'):r.apply(self.plan)
        self.assertEqual(self.snap(),before)
    def test_duplicate_json_is_refused_not_repaired(self):
        c=sqlite3.connect(self.path);c.execute('UPDATE config_profiles SET settings_json=?',('{"modelApiKey":"a","modelApiKey":"b","strixLlm":"old"}',));c.commit();c.close();shutil.copyfile(self.path,self.backup)
        before=self.snap()
        with self.assertRaisesRegex(ValueError,'duplicate_json'):r.inventory(self.path,self.backup)
        self.assertEqual(self.snap(),before)

if __name__=='__main__':unittest.main(verbosity=2)
