#!/usr/bin/env python3
"""Boundary tests for the read-only receipt aggregator; no network/model calls."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("cleanup_report", Path(__file__).with_name("cleanup-report.py"))
reporter = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reporter)

SCHEMA = """
CREATE TABLE asset_cleanup_runs(id TEXT PRIMARY KEY,project_id INTEGER,organization TEXT,phase TEXT,enabled INTEGER,raw_count INTEGER,ledger_count INTEGER,cursor INTEGER,reason TEXT,created_at INTEGER,updated_at INTEGER,roots_json TEXT);
CREATE TABLE asset_cleanup_groups(run_id TEXT,group_key TEXT,domain TEXT,state TEXT,shared INTEGER,advice TEXT,reason TEXT,evidence_json TEXT);
CREATE TABLE asset_cleanup_research(id INTEGER PRIMARY KEY,run_id TEXT,group_key TEXT,kind TEXT,state TEXT,attempts INTEGER,error TEXT,next_at INTEGER);
CREATE TABLE asset_cleanup_seeds(run_id TEXT,source_kind TEXT,source_id INTEGER,host TEXT,group_key TEXT,source TEXT);
CREATE TABLE asset_cleanup_batches(id TEXT PRIMARY KEY,run_id TEXT,project_id INTEGER,domains_json TEXT,restored INTEGER,created_at INTEGER);
CREATE TABLE asset_discovery_seeds(id INTEGER PRIMARY KEY,project_id INTEGER,host TEXT,source TEXT,source_ref TEXT);
CREATE TABLE asset_project_seeds(id INTEGER PRIMARY KEY,project_id INTEGER);
CREATE TABLE asset_discovery_domain_exclusions(project_id INTEGER,domain TEXT,enabled INTEGER);
CREATE TABLE asset_discovery_excluded_evidence(project_id INTEGER,host TEXT,source TEXT,source_ref TEXT,domain TEXT);
CREATE TABLE asset_discovery_seed_evidence(project_id INTEGER,host TEXT,source TEXT,source_ref TEXT);
"""


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "db with # question?.sqlite3"
        self.c = sqlite3.connect(self.path)
        self.c.executescript(SCHEMA)
        self.c.execute("INSERT INTO asset_cleanup_runs VALUES('one',1,'Target','completed_with_gaps',0,9,3,18,'pass done',10,11,'[\"target.test\"]')")
        self.c.execute("INSERT INTO asset_cleanup_runs VALUES('two',2,'Other','completed',0,1,1,2,'',20,21,'[]')")
        self.c.execute("INSERT INTO asset_cleanup_runs VALUES('older',1,'Target','paused',0,1,0,1,'',1,2,'[]')")

    def tearDown(self):
        self.c.close()
        self.temp.cleanup()

    def read(self, project=1, run="one", max_cases=5):
        self.c.commit()
        return reporter.read_report(self.path, project, run, max_cases)

    def group(self, key, state, shared=0, ev=None):
        self.c.execute("INSERT INTO asset_cleanup_groups VALUES('one',?,?,?,?,'','reason',?)", (key, key, state, shared, json.dumps(ev or {})))

    def test_read_only_special_uri_path_and_all_files_unchanged(self):
        self.group("outside.test", "deferred", ev={"completionReason": "network_exhausted"})
        self.c.commit()
        before = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in Path(self.temp.name).iterdir()}
        report = self.read()
        after = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in Path(self.temp.name).iterdir()}
        self.assertEqual(before, after)
        self.assertTrue(report["readOnly"])
        self.assertTrue(report["database"]["queryOnly"])
        self.assertEqual(report["database"]["uriMode"], "ro")
        self.assertFalse(report["completed"])
        self.assertTrue(report["passFinished"])
        self.assertEqual(report["research"]["remainingReasons"][0]["reason"], "network_exhausted")
        with sqlite3.connect(self.path.as_uri() + "?mode=ro", uri=True) as c:
            c.execute("PRAGMA query_only=ON")
            with self.assertRaises(sqlite3.OperationalError):
                c.execute("DELETE FROM asset_cleanup_runs")

    def test_same_group_keys_cross_run_and_project_do_not_leak(self):
        self.group("outside.test", "excluded")
        self.c.execute("INSERT INTO asset_cleanup_groups VALUES('two','outside.test','outside.test','conflict',0,'','private-other-project','{}')")
        self.c.execute("INSERT INTO asset_cleanup_research VALUES(1,'two','outside.test','ct_search','exhausted',3,'other project secret',1)")
        self.c.execute("INSERT INTO asset_cleanup_batches VALUES('wrong-project','one',2,'[\"outside.test\"]',0,1)")
        self.c.execute("INSERT INTO asset_cleanup_batches VALUES('other-run','older',1,'[\"outside.test\"]',0,1)")
        self.c.execute("INSERT INTO asset_discovery_seeds VALUES(1,2,'outside.test','history','asset:1')")
        r = self.read()
        self.assertEqual(r["groupStates"], {"excluded": 1})
        self.assertEqual(r["exclusions"]["newRunBatchCount"], 0)
        self.assertEqual(r["inventory"]["currentRawIndexRows"], 0)
        self.assertEqual(r["research"]["byKind"], {})
        self.assertNotIn("other project", json.dumps(r))
        with self.assertRaisesRegex(ValueError, "cleanup_run_not_found_for_project"):
            self.read(1, "two")
        self.assertEqual(self.read(1, None)["run"]["id"], "one")

    def test_exact_suffix_union_and_shared_fingerprint_provenance_guard(self):
        self.group("outside.test", "excluded")
        self.c.execute("INSERT INTO asset_cleanup_batches VALUES('new','one',1,'[\"outside.test\",\"child.outside.test\"]',0,1)")
        self.c.executemany("INSERT INTO asset_discovery_domain_exclusions VALUES(1,?,1)", [("outside.test",), ("child.outside.test",), ("historic.test",)])
        raw = [(1,1,"outside.test","history","asset:1"), (2,1,"a.outside.test","history","asset:2"), (3,1,"notoutside.test","history","asset:3"), (4,1,"outside.test.evil.test","history","asset:4"), (5,1,"a"*64,"certificate_sha256","asset:5"), (6,1,"b"*64,"certificate_sha256","asset:6"), (7,1,"child.outside.test","history","asset:7")]
        self.c.executemany("INSERT INTO asset_discovery_seeds VALUES(?,?,?,?,?)", raw)
        self.c.executemany("INSERT INTO asset_discovery_seed_evidence VALUES(1,?,?,?)", [("a"*64,"certificate_sha256","asset:5"), ("a"*64,"certificate_sha256","asset:outside-independent"), ("b"*64,"certificate_sha256","asset:6")])
        self.c.executemany("INSERT INTO asset_discovery_excluded_evidence VALUES(1,?,?,?,?)", [("a"*64,"certificate_sha256","asset:5","outside.test"), ("b"*64,"certificate_sha256","asset:6","outside.test")])
        r = self.read()
        ex = r["exclusions"]
        self.assertEqual(ex["coveredOriginalRawIndexRowsDirect"], 3)
        self.assertEqual(ex["coveredOriginalRawIndexRowsViaProvenance"], 1)
        self.assertEqual(ex["coveredOriginalRawIndexRowsUnion"], 4)
        self.assertEqual(ex["historicalEnabledDomainRulesOutsideRun"], 1)
        self.assertEqual(ex["activeBatchDomainCount"], 2)
        self.assertEqual(r["groupStates"]["excluded"], 1)  # 1 group != 4 rows.

    def test_restored_batch_and_shared_platform_root_warning(self):
        self.c.execute("INSERT INTO asset_cleanup_batches VALUES('old','one',1,'[\"restored.test\"]',1,1)")
        self.c.execute("INSERT INTO asset_cleanup_batches VALUES('unsafe','one',1,'[\"cmecloud.cn\"]',0,2)")
        self.c.executemany("INSERT INTO asset_discovery_domain_exclusions VALUES(1,?,1)", [("restored.test",), ("cmecloud.cn",)])
        self.c.execute("INSERT INTO asset_discovery_seeds VALUES(1,1,'a.cmecloud.cn','history','asset:1')")
        r = self.read()
        self.assertEqual(r["exclusions"]["restoredBatchCount"], 1)
        self.assertEqual(r["exclusions"]["sharedRootRuleWarnings"], ["cmecloud.cn"])
        self.assertEqual(r["exclusions"]["coveredOriginalRawIndexRowsDirect"], 1)
        self.assertNotIn("restored.test", r["exclusions"]["batches"][1]["activeDomains"])

    def test_future_states_and_deduplicated_provider_error_counts(self):
        self.group("a.test", "deferred", ev={"completionReason":"identity_missing_after_bounded_pass"})
        self.group("b.test", "evidence")
        self.group("c.test", "conflict")
        self.c.executemany("INSERT INTO asset_cleanup_research VALUES(?,'one',?,?,?,?,?,1)", [(1,"a.test","dns_cname","exhausted",3,"DNS Status=2 SERVFAIL: a.test"), (2,"b.test","dns_cname","retry",1,"DNS Status=2 SERVFAIL: b.test"), (3,"b.test","ct_search","pending",0,""), (4,"c.test","public_ownership","skipped",0,"shared tenant")])
        r = self.read(max_cases=1)
        self.assertEqual(r["research"]["groupsWithExhaustedWork"], 1)
        self.assertEqual(r["research"]["byKind"]["public_ownership"]["skipped"], 1)
        self.assertEqual(r["research"]["firstPassGroupsAttempted"], 2)
        self.assertEqual(r["research"]["firstPassGroupsNeverAttempted"], 1)
        dns = next(e for e in r["research"]["errorsByProviderKindReason"] if e["provider"] == "dns")
        self.assertEqual(dns["workItems"], 2)
        self.assertEqual(dns["reason"], "dns_servfail")
        self.assertEqual(r["cases"]["unresolved"]["totalGroups"], 2)
        self.assertTrue(r["cases"]["unresolved"]["truncated"])
        self.assertEqual(len(r["conflictGroups"]), 1)
        self.assertFalse(r["completed"])

    def test_secrets_certificate_raw_and_full_request_are_never_exported(self):
        self.group("a.test", "excluded", ev={"request":{"api-key":"do-not-leak"}, "rawCertificate":"-----BEGIN CERTIFICATE-----", "domainEvidence":[{"host":"a.test", "organization":"Other Limited", "source":"official_site_operator", "sourceUrl":"https://user:password@a.test/about?api-key=secret-key#token", "bodySha256":"a"*64, "rawCertificate":"do-not-export"}]})
        self.c.execute("INSERT INTO asset_cleanup_research VALUES(1,'one','a.test','public_ownership','retry',2,'HTTP 403 https://a.test/?api-key=leaky_key authorization=BearerSecret',1)")
        r = self.read()
        out = json.dumps(r) + reporter.markdown(r)
        for forbidden in ("do-not-leak", "do-not-export", "BEGIN CERTIFICATE", "leaky_key", "BearerSecret", "user:password", "secret-key", "#token"):
            self.assertNotIn(forbidden, out)
        self.assertIn("https://a.test/about", out)

    def test_empty_schema_and_partial_schema_do_not_claim_completion(self):
        empty = Path(self.temp.name) / "empty.sqlite3"
        sqlite3.connect(empty).close()
        r = reporter.read_report(empty, 1)
        self.assertEqual(r["availability"], "no_cleanup_run")
        self.assertIsNone(r["inventory"]["currentRawIndexRows"])
        self.assertFalse(r["completed"])
        self.c.execute("DROP TABLE asset_cleanup_research")
        r = self.read()
        self.assertEqual(r["availability"], "partial_schema")
        self.assertFalse(r["passFinished"])
        self.assertFalse(r["completed"])
        self.assertIn("asset_cleanup_research", r["missingSchema"])

    def test_legacy_missing_required_columns_are_unknown_not_zero(self):
        self.c.execute("DROP TABLE asset_discovery_seeds")
        self.c.execute("CREATE TABLE asset_discovery_seeds(host TEXT)")
        r = self.read()
        self.assertIsNone(r["inventory"]["currentRawIndexRows"])
        self.assertTrue(any(k.startswith("asset_discovery_seeds:") for k in r["missingSchema"]))
        self.assertFalse(r["completed"])

    def test_checkpointed_no_wal_uses_immutable_with_matching_file_stamps(self):
        self.c.commit()
        self.assertFalse(reporter.wal_present(self.path))
        r = self.read()
        self.assertTrue(r["database"]["immutable"])
        self.assertEqual(r["database"]["snapshotPolicy"], "stable_no_wal_immutable")
        self.assertEqual(r["database"]["fileStampBefore"], r["database"]["fileStampAfter"])
        self.assertFalse(reporter.wal_present(self.path))

    def test_wal_appearing_during_immutable_read_rejects_report(self):
        self.c.commit()
        original = reporter.build_report
        wal = Path(str(self.path) + "-wal")
        def new_wal(*args, **kwargs):
            result = original(*args, **kwargs)
            wal.touch()  # Even size zero must invalidate the no-WAL boundary.
            return result
        with mock.patch.object(reporter, "build_report", side_effect=new_wal):
            with self.assertRaisesRegex(ValueError, "changed_during_immutable_read"):
                self.read()
        self.assertEqual(wal.stat().st_size, 0)
        wal.unlink()

    def test_main_file_changed_during_immutable_read_rejects_report(self):
        self.c.commit()
        original = reporter.build_report
        def changed_database(*args, **kwargs):
            result = original(*args, **kwargs)
            before = self.path.stat()
            os.utime(self.path, ns=(before.st_atime_ns, before.st_mtime_ns + 1_000_000))
            return result
        with mock.patch.object(reporter, "build_report", side_effect=changed_database):
            with self.assertRaisesRegex(ValueError, "changed_during_immutable_read"):
                self.read()

    def test_main_inode_replaced_during_immutable_read_rejects_report(self):
        self.c.commit()
        original = reporter.build_report
        def replaced_database(*args, **kwargs):
            result = original(*args, **kwargs)
            replacement = Path(self.temp.name) / "replacement.sqlite3"
            replacement.write_bytes(self.path.read_bytes())
            os.replace(replacement, self.path)
            return result
        with mock.patch.object(reporter, "build_report", side_effect=replaced_database):
            with self.assertRaisesRegex(ValueError, "changed_during_immutable_read"):
                self.read()

    def test_present_live_wal_reads_latest_committed_rows_without_immutable(self):
        self.c.commit()
        self.assertEqual(self.c.execute("PRAGMA journal_mode=WAL").fetchone()[0], "wal")
        self.c.execute("INSERT INTO asset_discovery_seeds VALUES(987,1,'latest.test','history','asset:987')")
        self.c.commit()
        self.assertTrue(reporter.wal_present(self.path))
        r = self.read()
        self.assertFalse(r["database"]["immutable"])
        self.assertEqual(r["database"]["snapshotPolicy"], "live_wal_read_transaction")
        self.assertEqual(r["inventory"]["currentRawIndexRows"], 1)
        self.assertIsNone(r["database"]["fileStampBefore"])
        self.assertIsNone(r["database"]["fileStampAfter"])

    def test_zero_byte_wal_and_ro_open_failure_never_fall_back_to_immutable(self):
        self.c.commit()
        wal = Path(str(self.path) + "-wal")
        wal.touch()
        with mock.patch.object(reporter.sqlite3, "connect", side_effect=sqlite3.OperationalError("unable to open database file")) as connection:
            with self.assertRaises(sqlite3.OperationalError):
                self.read()
            self.assertEqual(connection.call_count, 1)
            self.assertTrue(connection.call_args.args[0].endswith("?mode=ro"))
            self.assertNotIn("immutable", connection.call_args.args[0])
            self.assertIn("%23", connection.call_args.args[0])
            self.assertIn("%3F", connection.call_args.args[0])
        wal.unlink()

    def test_wal_appearing_during_initial_stat_rejects_before_connect(self):
        self.c.commit()
        original = reporter.database_stamp
        wal = Path(str(self.path) + "-wal")
        def raced_stamp(path):
            stamp = original(path)
            wal.touch()
            return stamp
        with mock.patch.object(reporter, "database_stamp", side_effect=raced_stamp), mock.patch.object(reporter.sqlite3, "connect") as connection:
            with self.assertRaisesRegex(ValueError, "wal_appeared_before_immutable_read"):
                self.read()
            connection.assert_not_called()
        wal.unlink()

    def test_private_output_permissions_and_query_only_enforcement(self):
        target = Path(self.temp.name) / "out" / "report.json"
        reporter.private_write(target, "{}\n")
        self.assertEqual(target.stat().st_mode & 0o777, 0o600)
        os.chmod(target, 0o644)
        reporter.private_write(target, "{\"next\":true}\n")
        self.assertEqual(target.stat().st_mode & 0o777, 0o600)
        with self.assertRaisesRegex(ValueError, "query_only"):
            reporter.build_report(self.c, 1)


if __name__ == "__main__":
    unittest.main()
