//! Git commit correlation — link commits to sessions.

use anyhow::Result;
use rusqlite::Connection;
use std::process::Command;

/// Scan recent git commits and link them to the active session.
pub fn scan_and_link(conn: &Connection, session_id: &str, project: &str) -> Result<usize> {
    // Get commits since session start
    let session_start: String = conn.query_row(
        "SELECT started_at FROM sessions WHERE id = ?1",
        [session_id],
        |row| row.get(0),
    )?;

    let output = Command::new("git")
        .args(["log", "--format=%H\t%s\t%ai", &format!("--since={session_start}"), "--no-merges"])
        .current_dir(project)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut count = 0;

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }

        let hash = parts[0];
        let message = parts[1];
        let timestamp = parts[2];

        // Count files changed
        let diff_output = Command::new("git")
            .args(["diff-tree", "--no-commit-id", "--name-only", "-r", hash])
            .current_dir(project)
            .output()
            .ok();
        let files_changed = diff_output
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count() as i64)
            .unwrap_or(0);

        conn.execute(
            "INSERT OR IGNORE INTO commits (hash, session_id, project, message, files_changed, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![hash, session_id, project, message, files_changed, timestamp],
        )?;
        count += 1;
    }

    Ok(count)
}

/// Get commit stats for a session.
pub fn session_commit_stats(conn: &Connection, session_id: &str) -> Result<(i64, i64)> {
    let (count, files): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(files_changed), 0) FROM commits WHERE session_id = ?1",
        [session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((count, files))
}
