// Shared rollback assertion: include every non-internal table.
pub(super) fn application_table_snapshot(
    connection: &rusqlite::Connection,
) -> Vec<(String, Vec<String>)> {
    let tables = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|name| {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT * FROM \"{}\" ORDER BY rowid",
                    name.replace('"', "\"\"")
                ))
                .unwrap();
            let count = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    let values = (0..count)
                        .map(|i| row.get::<_, rusqlite::types::Value>(i))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn assert_application_tables_unchanged(
    connection: &rusqlite::Connection,
    before: &[(String, Vec<String>)],
) {
    let after = application_table_snapshot(connection);
    let changed: Vec<_> = before
        .iter()
        .chain(after.iter())
        .filter_map(|(name, _)| {
            (before.iter().find(|(key, _)| key == name)
                != after.iter().find(|(key, _)| key == name))
            .then_some(name)
        })
        .collect();
    assert!(
        changed.is_empty(),
        "operation changed application tables: {changed:?}"
    );
}
