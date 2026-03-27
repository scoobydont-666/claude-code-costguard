//! Transcript parser — extracts token usage, tool calls, and metadata from
//! Claude Code JSONL transcript files.
//!
//! This is the ONLY reliable source of token data and complete tool usage.
//! Hook payloads don't carry usage info, and live tool hooks can miss events.

use anyhow::Result;
use rusqlite::Connection;

use crate::db;

/// Parsed transcript results for reporting.
pub struct TranscriptResult {
    pub message_count: i64,
    pub tool_count: i64,
    pub total_cost: f64,
    pub prompt_count: i64,
}

/// Parse a transcript JSONL file and update session, message, and tool_usage tables.
/// Idempotent — safe to call multiple times on the same transcript.
pub fn parse(conn: &Connection, session_id: &str, path: &str) -> Result<TranscriptResult> {
    let content = std::fs::read_to_string(path)?;

    let mut total_input: i64 = 0;
    let mut total_output: i64 = 0;
    let mut total_cache_read: i64 = 0;
    let mut total_cache_write: i64 = 0;
    let mut model_name = String::new();
    let mut message_count: i64 = 0;
    let mut tool_count: i64 = 0;
    let mut prompt_count: i64 = 0;
    let mut first_timestamp: Option<String> = None;
    let mut last_timestamp: Option<String> = None;
    let mut session_cwd: Option<String> = None;
    let mut session_branch: Option<String> = None;

    // Clear existing tool_usage from transcript (we'll re-insert — idempotent)
    conn.execute(
        "DELETE FROM tool_usage WHERE session_id = ?1 AND message_id LIKE 'toolu_%'",
        [session_id],
    )?;

    for line in content.lines() {
        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Extract session metadata from first entry
        if first_timestamp.is_none() {
            first_timestamp = entry.get("timestamp").and_then(|v| v.as_str()).map(String::from);
            session_cwd = entry.get("cwd").and_then(|v| v.as_str()).map(String::from);
            session_branch = entry.get("gitBranch").and_then(|v| v.as_str()).map(String::from);
        }
        // Track last timestamp for session end time
        if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
            last_timestamp = Some(ts.to_string());
        }

        // Count user prompts
        if entry_type == "human" || entry_type == "user" {
            prompt_count += 1;
        }

        if entry_type == "assistant" {
            if let Some(msg) = entry.get("message") {
                // Extract model (use most recent non-synthetic model)
                if let Some(m) = msg.get("model").and_then(|v| v.as_str()) {
                    if !m.is_empty() && !m.starts_with('<') {
                        model_name = m.to_string();
                    }
                }

                // Extract usage
                if let Some(usage) = msg.get("usage") {
                    let input = usage.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    let output = usage.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    let cache_read = usage.get("cache_read_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    let cache_write = usage.get("cache_creation_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);

                    total_input += input;
                    total_output += output;
                    total_cache_read += cache_read;
                    total_cache_write += cache_write;
                    message_count += 1;

                    let cost = db::compute_cost(conn, &model_name, input, output, cache_read, cache_write);

                    let uuid = entry.get("uuid").and_then(|v| v.as_str()).unwrap_or("");
                    let timestamp = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                    if !uuid.is_empty() {
                        conn.execute(
                            "INSERT OR REPLACE INTO messages (id, session_id, timestamp, role, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            rusqlite::params![uuid, session_id, timestamp, "assistant", model_name, input, output, cache_read, cache_write, cost],
                        )?;
                    }
                }

                // Extract tool_use blocks from message content
                let timestamp = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(content_arr) = msg.get("content").and_then(|v| v.as_array()) {
                    for block in content_arr {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                            let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");

                            if !tool_id.is_empty() {
                                // Extract file path for file-oriented tools
                                let file_path = block.get("input").and_then(|input| {
                                    match tool_name {
                                        "Read" | "Write" | "Edit" => input.get("file_path").and_then(|v| v.as_str()),
                                        "Glob" => input.get("pattern").and_then(|v| v.as_str()),
                                        "Grep" => input.get("path").and_then(|v| v.as_str()),
                                        _ => None,
                                    }
                                });

                                conn.execute(
                                    "INSERT OR IGNORE INTO tool_usage (message_id, session_id, tool_name, timestamp, duration_ms, file_path) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                                    rusqlite::params![tool_id, session_id, tool_name, timestamp, file_path],
                                )?;
                                tool_count += 1;
                            }

                            // Track skill activations from transcript
                            if tool_name == "Skill" {
                                if let Some(input_val) = block.get("input") {
                                    let skill_name = input_val.get("skill")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown");
                                    conn.execute(
                                        "INSERT OR IGNORE INTO skill_activations (session_id, skill_name, activation_type, timestamp) VALUES (?1, ?2, ?3, ?4)",
                                        rusqlite::params![session_id, skill_name, "explicit", timestamp],
                                    ).ok();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Update session totals (overwrite, not accumulate — idempotent)
    let total_cost = db::compute_cost(conn, &model_name, total_input, total_output, total_cache_read, total_cache_write);
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE sessions SET model = COALESCE(NULLIF(?1, ''), model), total_input_tokens = ?2, total_output_tokens = ?3, total_cache_read = ?4, total_cache_write = ?5, total_cost_usd = ?6, started_at = COALESCE(?7, started_at), ended_at = COALESCE(?8, ended_at), project = COALESCE(NULLIF(?9, ''), project), git_branch = COALESCE(NULLIF(?10, ''), git_branch), last_synced_at = ?11, prompt_count = ?12 WHERE id = ?13",
        rusqlite::params![model_name, total_input, total_output, total_cache_read, total_cache_write, total_cost, first_timestamp, last_timestamp, session_cwd, session_branch, now, prompt_count, session_id],
    )?;

    Ok(TranscriptResult { message_count, tool_count, total_cost, prompt_count })
}

/// Fast token summation from a transcript file — no DB writes.
/// Used by statusline for live session data. Returns (input, output, cache_read, cache_write).
pub fn quick_sum_tokens(path: &str) -> Result<(i64, i64, i64, i64)> {
    let content = std::fs::read_to_string(path)?;
    let mut input = 0i64;
    let mut output = 0i64;
    let mut cache_read = 0i64;
    let mut cache_write = 0i64;

    for line in content.lines() {
        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if entry.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        if let Some(usage) = entry.get("message").and_then(|m| m.get("usage")) {
            input += usage.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            output += usage.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            cache_read += usage.get("cache_read_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            cache_write += usage.get("cache_creation_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
        }
    }
    Ok((input, output, cache_read, cache_write))
}

/// Discover transcript files from ~/.claude/projects/ and sync all unsynced sessions.
/// Uses `last_synced_at` for stale detection — sessions without a successful sync are re-parsed.
/// Acquires a sync lock to prevent concurrent sync operations.
pub fn sync_all(conn: &Connection, force: bool) -> Result<Vec<(String, TranscriptResult)>> {
    let mut results = Vec::new();

    // Acquire sync lock to prevent concurrent syncs
    if !db::try_acquire_sync_lock(conn) {
        anyhow::bail!("Another sync is in progress");
    }

    let sync_result = sync_all_inner(conn, force, &mut results);

    // Always release the lock
    db::release_sync_lock(conn);

    sync_result?;
    Ok(results)
}

fn sync_all_inner(conn: &Connection, force: bool, results: &mut Vec<(String, TranscriptResult)>) -> Result<()> {
    let claude_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".claude/projects");

    if !claude_dir.exists() {
        return Ok(());
    }

    for project_entry in std::fs::read_dir(&claude_dir)? {
        let project_entry = project_entry?;
        let project_path = project_entry.path();
        if !project_path.is_dir() {
            continue;
        }

        for file_entry in std::fs::read_dir(&project_path)? {
            let file_entry = file_entry?;
            let file_path = file_entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            let session_id = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if session_id.is_empty() {
                continue;
            }

            let session_exists: bool = conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0),
            ).unwrap_or(0) > 0;

            let needs_sync = if force {
                true
            } else {
                // Use last_synced_at instead of token count — catches partial parses too
                conn.query_row(
                    "SELECT last_synced_at FROM sessions WHERE id = ?1",
                    [&session_id],
                    |row| row.get::<_, Option<String>>(0),
                ).ok().flatten().is_none()
            };

            if !session_exists {
                let hostname = hostname::get()
                    .map(|h| h.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                let now = chrono::Utc::now().to_rfc3339();

                conn.execute(
                    "INSERT OR IGNORE INTO sessions (id, hostname, project, started_at, transcript_path) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![session_id, hostname, "", now, file_path.to_str().unwrap_or("")],
                )?;
            }

            // Always update transcript_path — filesystem is ground truth
            conn.execute(
                "UPDATE sessions SET transcript_path = ?1 WHERE id = ?2",
                rusqlite::params![file_path.to_str().unwrap_or(""), session_id],
            )?;

            if needs_sync || !session_exists {
                match parse(conn, &session_id, file_path.to_str().unwrap_or("")) {
                    Ok(result) => {
                        if result.message_count > 0 {
                            results.push((session_id, result));
                        }
                    }
                    Err(e) => {
                        db::log_sync_error(conn, &session_id, &e.to_string());
                        continue;
                    }
                }
            }
        }
    }

    Ok(())
}
