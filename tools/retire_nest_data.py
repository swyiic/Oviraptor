"""Explicit retirement plans. No database initialization or CAS deletion."""
import hashlib
from contextlib import closing
import json
import os
import sqlite3
from pathlib import Path

RETIRED_TABLES = ('strix_knowledge_entries', 'strix_learning_candidates', 'strix_skills')
CONFIG_ALIASES = {'modelLocalFullPower', 'modelPromptAuditMode', 'legacyArtifactDirectories',
                  'historicalImportDirectories', 'agentBackendPolicy'}
ALLOWED = set(RETIRED_TABLES) | {'app_settings', 'config_profiles', 'sentinel_checkpoints', 'sentinel_findings'}


def quote(name):
    return '"' + name.replace('"', '""') + '"'


def connect(path, writable=False):
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError('retirement_database_not_regular')
    db = sqlite3.connect(path.as_uri() + ('?mode=rw' if writable else '?mode=ro'), uri=True, timeout=5)
    db.execute('PRAGMA foreign_keys=ON')
    if not writable:
        db.execute('PRAGMA query_only=ON')
    return db


def encode(value):
    if value is None:
        return ['null']
    if isinstance(value, bytes):
        return ['blob', value.hex()]
    return [type(value).__name__, value]


def digest_rows(rows):
    digest = hashlib.sha256()
    count = 0
    for row in rows:
        data = json.dumps([encode(v) for v in row], ensure_ascii=False, separators=(',', ':')).encode()
        digest.update(len(data).to_bytes(8, 'big'))
        digest.update(data)
        count += 1
    return {'count': count, 'sha256': digest.hexdigest()}


def tables(db):
    return dict(db.execute("SELECT name,sql FROM sqlite_master WHERE type='table' ORDER BY name"))


def rows(db, table):
    # Refuse unsupported storage rather than inventing a row identity or scope.
    if 'WITHOUT ROWID' in (tables(db)[table] or '').upper():
        raise ValueError('retirement_without_rowid_requires_review')
    return db.execute('SELECT rowid,* FROM ' + quote(table) + ' ORDER BY rowid')


def snapshot(db):
    schema = digest_rows(db.execute('SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name'))
    return {'schema': schema, 'tables': {t: digest_rows(rows(db, t)) for t in tables(db)}}


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as file:
        for data in iter(lambda: file.read(1024 * 1024), b''):
            digest.update(data)
    return digest.hexdigest()


def strict_object(raw):
    def pairs(values):
        obj = {}
        for key, value in values:
            if key in obj:
                raise ValueError('retirement_duplicate_json_key')
            obj[key] = value
        return obj
    def invalid(value):
        raise ValueError('retirement_non_json_number')
    value = json.loads(raw, object_pairs_hook=pairs, parse_constant=invalid)
    if not isinstance(value, dict):
        raise ValueError('retirement_settings_not_object')
    return value


def selection(db):
    existing = tables(db)
    if not ALLOWED <= set(existing):
        raise ValueError('retirement_schema_missing_scope')
    actions = []
    expected = {}
    for table in sorted(ALLOWED):
        columns = ['_rowid'] + [r[1] for r in db.execute('PRAGMA table_info(' + quote(table) + ')')]
        keep = []
        for record in rows(db, table):
            value = dict(zip(columns, record))
            retire = table in RETIRED_TABLES
            replacement = None
            removed = []
            if table == 'app_settings':
                retire = value['key'].lower().startswith('strix')
            elif table == 'sentinel_checkpoints':
                stage = value['stage']
                retire = stage in ('strix_run', 'strix_events', 'strix_coverage') or stage.startswith(
                    ('strix_run:', 'strix_events:', 'strix_coverage:'))
            elif table == 'sentinel_findings':
                retire = value['stage'] in ('strix', 'strix-coverage')
            elif table == 'config_profiles':
                original = strict_object(value['settings_json'])
                removed = [k for k in original if k.lower().startswith('strix') or k in CONFIG_ALIASES]
                if removed:
                    current = {k: v for k, v in original.items() if k not in removed}
                    replacement = json.dumps(current, ensure_ascii=False, separators=(',', ':'), allow_nan=False)
            if retire or replacement is not None:
                action = {'table': table, 'rowid': record[0], 'original_hash': digest_rows([record])['sha256'],
                          'kind': 'delete' if retire else 'settings', 'removed_keys': removed}
                actions.append(action)
            if not retire:
                if replacement is not None:
                    changed = list(record)
                    changed[columns.index('settings_json')] = replacement
                    keep.append(tuple(changed))
                else:
                    keep.append(record)
        expected[table] = digest_rows(keep)
    return actions, expected


def inventory(database, backup):
    database, backup = Path(database).resolve(strict=True), Path(backup).resolve(strict=True)
    if os.path.samefile(database, backup):
        raise ValueError('retirement_backup_is_live_database')
    with closing(connect(backup)) as clone:
        clone.execute('BEGIN')
        if clone.execute('PRAGMA quick_check').fetchall() != [('ok',)]:
            raise ValueError('retirement_backup_integrity_failed')
        saved = snapshot(clone)
        clone.rollback()
    with closing(connect(database)) as db:
        db.execute('BEGIN')
        before = snapshot(db)
        if before != saved:
            raise ValueError('retirement_backup_does_not_match_current_data')
        actions, expected = selection(db)
        db.rollback()
    return {'version': 1, 'database': str(database), 'backup': str(backup), 'backup_sha256': file_hash(backup),
            'before': before, 'actions': actions, 'expected_scope': expected}


def apply(plan):
    if type(plan.get('version')) is not int or plan['version'] != 1:
        raise ValueError('retirement_plan_version_invalid')
    backup = Path(plan['backup']).resolve(strict=True)
    database = Path(plan['database']).resolve(strict=True)
    if os.path.samefile(database, backup):
        raise ValueError('retirement_backup_is_live_database')
    if file_hash(backup) != plan['backup_sha256']:
        raise ValueError('retirement_backup_changed')
    # A valid checksum alone cannot substitute a different backup's contents.
    with closing(connect(backup)) as clone:
        clone.execute('BEGIN')
        if snapshot(clone) != plan['before']:
            raise ValueError('retirement_backup_scope_conflict')
        actions, expected = selection(clone)
        if actions != plan['actions'] or expected != plan['expected_scope']:
            raise ValueError('retirement_plan_scope_conflict')
        clone.rollback()
    target = dict(plan['before']['tables'])
    target.update(expected)
    completed = {'schema': plan['before']['schema'], 'tables': target}
    db = connect(database, writable=True)
    try:
        db.execute('BEGIN IMMEDIATE')
        before = snapshot(db)
        if before == completed and before != plan['before']:
            if file_hash(backup) != plan['backup_sha256'] or db.total_changes != 0:
                raise ValueError('retirement_replay_unconfirmed')
            db.rollback()
            return {'changed_rows': 0, 'already_applied': True, 'cas_mutated': False}
        if before != plan['before']:
            raise ValueError('retirement_source_changed_since_inventory')
        actions, expected = selection(db)
        if actions != plan['actions'] or expected != plan['expected_scope']:
            raise ValueError('retirement_plan_scope_conflict')
        violations = list(db.execute('PRAGMA foreign_key_check'))
        changes = db.total_changes
        # An injected trigger must never even attempt to write assets or any
        # table outside this explicit plan. Final snapshots also catch edits
        # to current rows within the seven candidate tables.
        def authorize(action, arg1, arg2, database_name, trigger):
            if action in (sqlite3.SQLITE_INSERT, sqlite3.SQLITE_UPDATE, sqlite3.SQLITE_DELETE):
                return sqlite3.SQLITE_OK if arg1 in ALLOWED and database_name == 'main' else sqlite3.SQLITE_DENY
            if action in (sqlite3.SQLITE_ATTACH, sqlite3.SQLITE_DETACH, sqlite3.SQLITE_ALTER_TABLE,
                          sqlite3.SQLITE_DROP_TABLE, sqlite3.SQLITE_CREATE_TABLE):
                return sqlite3.SQLITE_DENY
            return sqlite3.SQLITE_OK
        db.set_authorizer(authorize)
        for action in actions:
            table, rowid = action['table'], action['rowid']
            if action['kind'] == 'delete':
                changed = db.execute('DELETE FROM ' + quote(table) + ' WHERE rowid=?', (rowid,)).rowcount
            else:
                raw = db.execute('SELECT settings_json FROM config_profiles WHERE rowid=?', (rowid,)).fetchone()[0]
                original = strict_object(raw)
                current = {k: v for k, v in original.items() if k not in action['removed_keys']}
                raw = json.dumps(current, ensure_ascii=False, separators=(',', ':'), allow_nan=False)
                changed = db.execute('UPDATE config_profiles SET settings_json=? WHERE rowid=?', (raw, rowid)).rowcount
            if changed != 1:
                raise ValueError('retirement_row_write_unconfirmed')
        target = dict(before['tables'])
        target.update(expected)
        after = snapshot(db)
        if after['schema'] != before['schema'] or after['tables'] != target:
            raise ValueError('retirement_final_data_scope_changed')
        if db.total_changes - changes != len(actions):
            raise ValueError('retirement_unplanned_writes')
        if list(db.execute('PRAGMA foreign_key_check')) != violations:
            raise ValueError('retirement_foreign_key_scope_changed')
        if file_hash(backup) != plan['backup_sha256']:
            raise ValueError('retirement_backup_changed_during_write')
        db.commit()
        return {'changed_rows': len(actions), 'already_applied': False, 'changed_tables': sorted({a['table'] for a in actions}),
                'protected_tables': len(before['tables']) - len(ALLOWED), 'cas_mutated': False}
    except BaseException:
        db.rollback()
        raise
    finally:
        db.close()


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    preview = commands.add_parser('plan', help='Read an existing DB and matching verified backup; do not mutate either')
    preview.add_argument('--database', required=True, type=Path)
    preview.add_argument('--backup', required=True, type=Path)
    preview.add_argument('--output', required=True, type=Path)
    execute = commands.add_parser('apply', help='Apply only the exact frozen plan, preserving all other data')
    execute.add_argument('--plan', required=True, type=Path)
    args = parser.parse_args()
    if args.command == 'plan':
        plan = inventory(args.database, args.backup)
        fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as output:
            json.dump(plan, output, ensure_ascii=False, indent=2)
        report = {'planned_rows': len(plan['actions']), 'plan': str(args.output), 'database_mutated': False}
    else:
        plan = strict_object(args.plan.read_text())
        report = apply(plan)
    print(json.dumps(report, ensure_ascii=False))


if __name__ == '__main__':
    main()
