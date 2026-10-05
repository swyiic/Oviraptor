const TIMELINE_EVENT_PAGE_SIZE: i64 = 100;

// Bound the rows projected by status independently of the scan's execution
// snapshot. Cursors are scoped to an attempt and are never execution leases.
struct TimelineWindow {
    after: i64,
    before: Option<i64>,
    watermark: i64,
    first_sequence: i64,
    has_earlier: bool,
    has_newer: bool,
}

impl TimelineWindow {
    fn latest(
        connection: &rusqlite::Connection,
        scan_id: &str,
        attempt: i64,
        after: Option<i64>,
    ) -> Result<Self, String> {
        let latest: i64 = connection.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events \
             WHERE scan_id=?1 AND attempt_number=?2",
            params![scan_id, attempt], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let (lower, upper, watermark) = if let Some(cursor) = after {
            if cursor < 0 { return Err("无效的时间线游标".into()); }
            let hundredth: Option<i64> = connection.query_row(
                "SELECT sequence FROM agent_collaboration_events \
                 WHERE scan_id=?1 AND attempt_number=?2 AND sequence>?3 \
                 ORDER BY sequence LIMIT 1 OFFSET ?4",
                params![scan_id, attempt, cursor, TIMELINE_EVENT_PAGE_SIZE - 1],
                |row| row.get(0),
            ).optional().map_err(|error| error.to_string())?;
            let watermark = hundredth.unwrap_or(latest);
            (cursor, if watermark < latest { Some(watermark + 1) } else { None }, watermark)
        } else {
            let hundredth: Option<i64> = connection.query_row(
                "SELECT sequence FROM agent_collaboration_events \
                 WHERE scan_id=?1 AND attempt_number=?2 ORDER BY sequence DESC LIMIT 1 OFFSET ?3",
                params![scan_id, attempt, TIMELINE_EVENT_PAGE_SIZE - 1],
                |row| row.get(0),
            ).optional().map_err(|error| error.to_string())?;
            (hundredth.map_or(0, |sequence| sequence - 1), None, latest)
        };
        let mut window = Self::from_bounds(connection, scan_id, attempt, lower, upper, watermark)?;
        window.has_newer = watermark < latest;
        Ok(window)
    }

    fn older(
        connection: &rusqlite::Connection,
        scan_id: &str,
        attempt: i64,
        before: i64,
    ) -> Result<Self, String> {
        if before <= 0 { return Err("无效的历史时间线游标".into()); }
        let hundredth: Option<i64> = connection.query_row(
            "SELECT sequence FROM agent_collaboration_events \
             WHERE scan_id=?1 AND attempt_number=?2 AND sequence<?3 \
             ORDER BY sequence DESC LIMIT 1 OFFSET ?4",
            params![scan_id, attempt, before, TIMELINE_EVENT_PAGE_SIZE - 1],
            |row| row.get(0),
        ).optional().map_err(|error| error.to_string())?;
        Self::from_bounds(connection, scan_id, attempt,
            hundredth.map_or(0, |sequence| sequence - 1), Some(before), 0)
    }

    fn from_bounds(
        connection: &rusqlite::Connection,
        scan_id: &str,
        attempt: i64,
        after: i64,
        before: Option<i64>,
        watermark: i64,
    ) -> Result<Self, String> {
        let first_sequence: i64 = connection.query_row(
            "SELECT COALESCE(MIN(sequence),0) FROM agent_collaboration_events \
             WHERE scan_id=?1 AND attempt_number=?2 AND sequence>?3 \
               AND (?4 IS NULL OR sequence<?4)",
            params![scan_id, attempt, after, before], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let has_earlier = if first_sequence == 0 { false } else {
            connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_collaboration_events \
                 WHERE scan_id=?1 AND attempt_number=?2 AND sequence<?3)",
                params![scan_id, attempt, first_sequence], |row| row.get::<_, bool>(0),
            ).map_err(|error| error.to_string())?
        };
        Ok(Self { after, before, watermark, first_sequence, has_earlier, has_newer: false })
    }
}
