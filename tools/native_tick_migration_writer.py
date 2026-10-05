"""Restrict this connection's DDL and writes through commit to two receipt tables."""
import sqlite3 as sql
from native_tick_migration_schema import TABLES, GUARDS


def install(db):
    def authorize(action, first, second, database, source):
        direct = source is None
        schema = first in ('sqlite_master', 'sqlite_temp_master')
        copies = first in ('migration_0', 'migration_1') and database == 'temp'
        if action in (sql.SQLITE_READ, sql.SQLITE_SELECT, sql.SQLITE_FUNCTION, sql.SQLITE_TRANSACTION, sql.SQLITE_RECURSIVE):
            allowed = True
        elif action == sql.SQLITE_PRAGMA:
            allowed = first in ('table_info', 'table_list', 'foreign_key_check')
        elif action in (sql.SQLITE_INSERT, sql.SQLITE_DELETE, sql.SQLITE_UPDATE):
            allowed = direct and (schema or first in TABLES or copies)
        elif action in (sql.SQLITE_CREATE_TABLE, sql.SQLITE_DROP_TABLE):
            allowed = direct and database == 'main' and first in TABLES
        elif action == sql.SQLITE_CREATE_TEMP_TABLE:
            allowed = direct and copies
        elif action in (sql.SQLITE_CREATE_TRIGGER, sql.SQLITE_DROP_TRIGGER):
            allowed = direct and first in GUARDS and second in TABLES
        elif action in (sql.SQLITE_CREATE_INDEX, sql.SQLITE_DROP_INDEX, sql.SQLITE_REINDEX):
            allowed = direct and second in TABLES and first.startswith('sqlite_autoindex_')
        else:
            allowed = False
        return sql.SQLITE_OK if allowed else sql.SQLITE_DENY
    db.set_authorizer(authorize)
