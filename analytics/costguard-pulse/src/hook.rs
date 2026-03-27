//! costguard-pulse-hook — fast hook binary called by Claude Code settings.json
//!
//! Reads JSON from stdin (Claude Code hook payload), writes to SQLite.
//! Must be fast — Claude Code waits for hooks to complete (<50ms target).
//!
//! Token usage is NOT in hook payloads — it's only in the transcript JSONL.
//! Transcript parsing happens via detached `sync-session` subprocess or `costguard-pulse sync`.

use anyhow::Result;
use chrono::Utc;
use serde::Deserialize;
use std::env;
use std::io::Read;

mod db;
mod routing;
mod transcript;
mod commits;

/// Base payload shared by all Claude Code hooks.
/// Claude Code may send camelCase or snake_case — we accept both via aliases.
#[derive(Debug, Deserialize)]
struct HookPayload {
    #[serde(default, alias = "sessionId", alias = "session_id")]
    session_id: String,
    #[serde(default, alias = "transcriptPath", alias = "transcript_path")]
    transcript_path: Option<String>,
    #[serde(default)]
    cwd: Option<String>,

    // PostToolUse-specific
    #[serde(default, alias = "toolName", alias = "tool_name")]
    tool_name: Option<String>,
    #[serde(default, alias = "toolInput", alias = "tool_input")]
    tool_input: Option<serde_json::Value>,

    // SubagentStart/Stop-specific
    #[serde(default, alias = "agentType", alias = "agent_type")]
    agent_type: Option<String>,
    #[serde(default, alias = "agentId", alias = "agent_id")]
    agent_id: Option<String>,
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("unknown");

    // sync-session takes args from CLI, not stdin — used by fork-and-detach
    if command == "sync-session" {
        return cmd_sync_session(&args);
    }

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok();

    let payload: HookPayload = match serde_json::from_str(&input) {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };

    let conn = db::open()?;
    let now = Utc::now().to_rfc3339();
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let session_id = &payload.session_id;

    if session_id.is_empty() {
        return Ok(());
    }

    match command {
        "session-start" => {
            let cwd = payload.cwd.unwrap_or_default();
            let transcript = payload.transcript_path.unwrap_or_default();

            // Detect git branch from cwd
            let git_branch = std::process::Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .current_dir(if cwd.is_empty() { "." } else { &cwd })
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            // Capture task ID from environment (optional, e.g. set by dispatcher scripts)
            let swarm_task_id = env::var("SWARM_TASK_ID").unwrap_or_default();

            conn.execute(
                "INSERT OR IGNORE INTO sessions (id, hostname, project, started_at, git_branch, transcript_path, swarm_task_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![session_id, hostname, cwd, now, git_branch, transcript, if swarm_task_id.is_empty() { None::<&str> } else { Some(swarm_task_id.as_str()) }],
            )?;

            // Safety net: sync any previous sessions that never completed sync.
            // Uses last_synced_at — catches both zero-token and partial-parse sessions.
            let mut stmt = conn.prepare(
                "SELECT id, transcript_path FROM sessions WHERE id != ?1 AND transcript_path IS NOT NULL AND transcript_path != '' AND last_synced_at IS NULL"
            )?;
            let stale: Vec<(String, String)> = stmt.query_map([session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?.filter_map(|r| r.ok()).collect();

            for (stale_id, stale_path) in &stale {
                if let Err(e) = transcript::parse(&conn, stale_id, stale_path) {
                    db::log_sync_error(&conn, stale_id, &e.to_string());
                }
            }
        }

        "prompt" => {
            // UserPromptSubmit — prompt count tracked via transcript parsing.
        }

        "tool-use" => {
            if let Some(tool_name) = payload.tool_name {
                // Extract file path for file-oriented tools (real-time visibility)
                let file_path = payload.tool_input.as_ref().and_then(|input| {
                    match tool_name.as_str() {
                        "Read" | "Write" | "Edit" => input.get("file_path").and_then(|v| v.as_str()),
                        "Glob" => input.get("pattern").and_then(|v| v.as_str()),
                        "Grep" => input.get("path").and_then(|v| v.as_str()),
                        _ => None,
                    }
                });

                conn.execute(
                    "INSERT INTO tool_usage (message_id, session_id, tool_name, timestamp, duration_ms, file_path) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                    rusqlite::params!["", session_id, tool_name, now, file_path],
                )?;

                // Track skill activations
                if tool_name == "Skill" {
                    let skill_name = payload.tool_input
                        .as_ref()
                        .and_then(|v| v.get("skill"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    conn.execute(
                        "INSERT INTO skill_activations (session_id, skill_name, activation_type, timestamp) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![session_id, skill_name, "explicit", now],
                    )?;
                }
            }
        }

        "agent-start" => {
            let agent_id = payload.agent_id.unwrap_or_else(|| format!("agent-{}", Utc::now().timestamp_millis()));
            let agent_type = payload.agent_type.unwrap_or_else(|| "unknown".to_string());
            let (recommended, reason) = routing::recommend_model(&agent_type);

            conn.execute(
                "INSERT OR REPLACE INTO subagents (id, session_id, model, recommended_model, routing_reason, started_at, task_description) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![agent_id, session_id, "unknown", recommended, reason, now, agent_type],
            )?;
        }

        "agent-end" => {
            if let Some(agent_id) = payload.agent_id {
                conn.execute(
                    "UPDATE subagents SET ended_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, agent_id],
                )?;
            }
        }

        "session-end" => {
            conn.execute(
                "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
                rusqlite::params![now, session_id],
            )?;

            // Resolve transcript path
            let transcript_path = payload.transcript_path.or_else(|| {
                conn.query_row(
                    "SELECT transcript_path FROM sessions WHERE id = ?1",
                    [session_id],
                    |row| row.get::<_, Option<String>>(0),
                ).ok().flatten()
            });

            // Parse transcript: fork-and-detach by default so the hook returns fast.
            // COSTGUARD_PULSE_SYNC_INLINE=1 forces inline parsing (used in tests).
            if let Some(path) = transcript_path {
                if !path.is_empty() {
                    if env::var("COSTGUARD_PULSE_SYNC_INLINE").unwrap_or_default() == "1" {
                        // Inline mode: parse directly (deterministic for tests)
                        if let Err(e) = transcript::parse(&conn, session_id, &path) {
                            db::log_sync_error(&conn, session_id, &e.to_string());
                            eprintln!("costguard-pulse: transcript parse error: {e}");
                        }
                        // Auto-import git commits
                        let project: Option<String> = conn.query_row(
                            "SELECT project FROM sessions WHERE id = ?1",
                            [session_id],
                            |row| row.get(0),
                        ).ok().flatten();
                        if let Some(proj) = project {
                            if !proj.is_empty() && std::path::Path::new(&proj).join(".git").exists() {
                                commits::scan_and_link(&conn, session_id, &proj).ok();
                            }
                        }
                    } else {
                        // Fork-and-detach: spawn a detached process for transcript parsing.
                        // Returns immediately so the hook doesn't timeout on large transcripts.
                        let self_exe = env::current_exe().unwrap_or_else(|_| "costguard-pulse-hook".into());
                        match std::process::Command::new(&self_exe)
                            .args(["sync-session", session_id, &path])
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                        {
                            Ok(_) => {}
                            Err(e) => {
                                db::log_sync_error(&conn, session_id, &format!("spawn sync-session failed: {e}"));
                                // Fallback: inline parse (slower but prevents data loss)
                                if let Err(e2) = transcript::parse(&conn, session_id, &path) {
                                    db::log_sync_error(&conn, session_id, &e2.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        _ => {}
    }

    Ok(())
}

/// Detached sync-session subcommand — runs transcript parse + commit import.
/// Called via fork-and-detach from session-end hook. Takes args from CLI, not stdin.
fn cmd_sync_session(args: &[String]) -> Result<()> {
    let session_id = args.get(2).map(|s| s.as_str()).unwrap_or("");
    let path = args.get(3).map(|s| s.as_str()).unwrap_or("");

    if session_id.is_empty() || path.is_empty() {
        return Ok(());
    }

    let conn = db::open()?;

    // Parse transcript
    if let Err(e) = transcript::parse(&conn, session_id, path) {
        db::log_sync_error(&conn, session_id, &e.to_string());
        eprintln!("costguard-pulse: sync-session parse error: {e}");
    }

    // Auto-import git commits
    let project: Option<String> = conn.query_row(
        "SELECT project FROM sessions WHERE id = ?1",
        [session_id],
        |row| row.get(0),
    ).ok().flatten();

    if let Some(proj) = project {
        if !proj.is_empty() && std::path::Path::new(&proj).join(".git").exists() {
            commits::scan_and_link(&conn, session_id, &proj).ok();
        }
    }

    Ok(())
}
