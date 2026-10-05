fn enabled_rule_pack(connection: &rusqlite::Connection, engine: &str) -> Option<String> {
    connection.query_row("SELECT local_path FROM security_rule_packs WHERE engine=?1 AND enabled=1 AND status='ready' AND local_path<>'' ORDER BY builtin DESC,id LIMIT 1",[engine],|row|row.get(0)).optional().ok().flatten()
}
