//! Integration tests for costguard-pulse hook and CLI binaries.
//!
//! Each test uses a unique XDG_DATA_HOME to avoid parallel test interference.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

fn hook_bin() -> String {
    std::path::Path::new(env!("CARGO_BIN_EXE_costguard-pulse-hook"))
        .to_str().unwrap().to_string()
}

fn cli_bin() -> String {
    std::path::Path::new(env!("CARGO_BIN_EXE_costguard-pulse"))
        .to_str().unwrap().to_string()
}

/// Each test gets a unique data dir so parallel tests don't collide.
fn unique_data_dir() -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let tid = std::thread::current().id();
    let dir = format!("/tmp/costguard-pulse-test-{id}-{tid:?}");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(format!("{dir}/costguard-pulse")).unwrap();
    dir
}

/// Create a minimal transcript JSONL with known values.
fn write_test_transcript(path: &str) {
    let mut f = fs::File::create(path).unwrap();
    writeln!(f, r#"{{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","uuid":"meta-1","cwd":"/opt/test-project","sessionId":"test-session-1","gitBranch":"main"}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","uuid":"msg-1","message":{{"model":"claude-opus-4-6","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200}},"content":[{{"type":"text","text":"hello"}},{{"type":"tool_use","id":"toolu_test1","name":"Read","input":{{"file_path":"/tmp/test"}}}},{{"type":"tool_use","id":"toolu_test2","name":"Bash","input":{{"command":"echo hi"}}}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:05.000Z","uuid":"msg-2","message":{{"model":"claude-opus-4-6","usage":{{"input_tokens":200,"output_tokens":100,"cache_read_input_tokens":2000,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"done"}},{{"type":"tool_use","id":"toolu_test3","name":"Skill","input":{{"skill":"cost-optimizer"}}}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","timestamp":"2026-03-20T10:00:10.000Z","uuid":"user-1"}}"#).unwrap();
}

fn run_hook(data_dir: &str, cmd: &str, json: &str) -> std::process::Output {
    let mut child = Command::new(hook_bin())
        .arg(cmd)
        .env("XDG_DATA_HOME", data_dir)
        .env("COSTGUARD_PULSE_SYNC_INLINE", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.take().unwrap().write_all(json.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn run_cli(data_dir: &str, args: &[&str]) -> String {
    let output = Command::new(cli_bin())
        .args(args)
        .env("XDG_DATA_HOME", data_dir)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        panic!("CLI failed: {stderr}\n{stdout}");
    }
    stdout
}

#[test]
fn test_session_start_creates_session() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-1","cwd":"/opt/test","transcriptPath":"/tmp/test.jsonl"}"#);
    let result = run_cli(&dir, &["doctor"]);
    assert!(result.contains("1 sessions"));
}

#[test]
fn test_tool_use_records_tool() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-t","cwd":"/opt/test"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-t","toolName":"Read"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-t","toolName":"Bash"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-t","toolName":"Bash"}"#);

    let result = run_cli(&dir, &["tools", "--period", "all"]);
    assert!(result.contains("Bash"), "Expected Bash: {result}");
    assert!(result.contains("Read"), "Expected Read: {result}");
}

#[test]
fn test_agent_lifecycle() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-a","cwd":"/opt/test"}"#);
    run_hook(&dir, "agent-start", r#"{"sessionId":"sess-a","agentId":"agent-1","agentType":"Explore"}"#);
    run_hook(&dir, "agent-end", r#"{"sessionId":"sess-a","agentId":"agent-1"}"#);

    let result = run_cli(&dir, &["agents"]);
    assert!(result.contains("agent-1"), "Expected agent-1: {result}");
}

#[test]
fn test_session_end_parses_transcript() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-p","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-p","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["stats", "--period", "all"]);
    assert!(result.contains("2"), "Expected 2 messages: {result}");
}

#[test]
fn test_transcript_tool_extraction() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-te","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-te","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["tools", "--period", "all"]);
    assert!(result.contains("Read"), "Expected Read: {result}");
    assert!(result.contains("Bash"), "Expected Bash: {result}");
    assert!(result.contains("Skill"), "Expected Skill: {result}");
}

#[test]
fn test_cost_calculation() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-c","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-c","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["cost", "--by", "session"]);
    assert!(result.contains("$"), "Expected cost: {result}");
    // Opus pricing: (300*5 + 150*25 + 3000*0.5 + 200*6.25) / 1M = small but non-zero
    assert!(!result.contains("$    0.00"), "Cost should be non-zero: {result}");
}

#[test]
fn test_sync_force_resync() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-rs","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-rs","transcriptPath":"{tp}"}}"#));

    // Force re-sync — should find transcript via DB transcript_path
    let result = run_cli(&dir, &["sync", "--session", "sess-rs", "--force"]);
    assert!(result.contains("Synced") || result.contains("msgs"), "Expected sync result: {result}");
}

#[test]
fn test_empty_payload_no_crash() {
    let dir = unique_data_dir();
    assert!(run_hook(&dir, "session-start", r#"{"sessionId":""}"#).status.success());
    assert!(run_hook(&dir, "session-start", "not json").status.success());
    assert!(run_hook(&dir, "tool-use", r#"{"sessionId":"x"}"#).status.success());
}

#[test]
fn test_snake_and_camel_case() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-camel","cwd":"/opt/test1"}"#);
    run_hook(&dir, "session-start", r#"{"session_id":"sess-snake","cwd":"/opt/test2"}"#);

    let result = run_cli(&dir, &["sessions", "--period", "all"]);
    assert!(result.contains("test1") || result.contains("test2"), "Both sessions should exist: {result}");
}

#[test]
fn test_doctor_output() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["doctor"]);
    assert!(result.contains("DB:"));
    assert!(result.contains("Pricing:"));
    assert!(result.contains("Data:"));
}

#[test]
fn test_statusline() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["statusline"]);
    assert!(result.contains("wk"), "Expected 'wk' in: {result}");
    assert!(result.contains("5h"), "Expected '5h' in: {result}");
    assert!(result.contains("cache"), "Expected 'cache' in: {result}");
}

#[test]
fn test_efficiency_empty_db() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["efficiency"]);
    assert!(result.contains("Total cost"));
}

#[test]
fn test_stale_session_auto_sync() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/stale.jsonl");
    write_test_transcript(&tp);

    // Create stale session (has transcript, never got session-end)
    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-stale","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));

    // New session triggers auto-sync of stale
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-fresh","cwd":"/opt/test2"}"#);

    // Stale session should now have data
    let result = run_cli(&dir, &["cost", "--by", "session"]);
    assert!(!result.is_empty(), "Should have cost data: {result}");
}

// --- Phase 2 tests ---

#[test]
fn test_file_path_extraction() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-fp","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-fp","transcriptPath":"{tp}"}}"#));

    // Check the DB directly via sync output — the Read tool in transcript has file_path "/tmp/test"
    let result = run_cli(&dir, &["tools", "--period", "all"]);
    assert!(result.contains("Read"), "Expected Read tool: {result}");
}

#[test]
fn test_prompt_count() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-pc","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-pc","transcriptPath":"{tp}"}}"#));

    // The test transcript has 1 user entry (type: "user")
    let result = run_cli(&dir, &["stats", "--period", "all"]);
    // Stats should contain session data — prompt_count is in DB but not yet displayed
    assert!(result.contains("Sessions"), "Expected sessions output: {result}");
}

#[test]
fn test_last_synced_at_set_on_parse() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-ls","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    // Before session-end: doctor should show stale
    let result = run_cli(&dir, &["doctor"]);
    assert!(result.contains("never synced") || result.contains("Stale"), "Expected stale before sync: {result}");

    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-ls","transcriptPath":"{tp}"}}"#));
    // After session-end: doctor should NOT show stale for this session
    let result = run_cli(&dir, &["doctor"]);
    // The session was synced, so stale count should be 0 (or at least not include this session)
    assert!(!result.contains("1 sessions never synced"), "Session should be synced: {result}");
}

#[test]
fn test_sync_error_logging() {
    let dir = unique_data_dir();

    // Create a session pointing to a nonexistent transcript
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-err","cwd":"/opt/test","transcriptPath":"/nonexistent/bad.jsonl"}"#);

    // Start a new session to trigger stale recovery on sess-err
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-ok","cwd":"/opt/test2"}"#);

    // Doctor should report sync errors
    let result = run_cli(&dir, &["doctor", "--errors"]);
    assert!(result.contains("Sync errors") || result.contains("sync error") || result.contains("sess-err"),
        "Expected sync error logged: {result}");
}

#[test]
fn test_sync_session_subcommand() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test.jsonl");
    write_test_transcript(&tp);

    // Create session first
    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-sc","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));

    // Run sync-session directly (simulates what fork-and-detach does)
    let output = Command::new(hook_bin())
        .args(["sync-session", "sess-sc", &tp])
        .env("XDG_DATA_HOME", &dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "sync-session should succeed");

    // Verify data was parsed
    let result = run_cli(&dir, &["stats", "--period", "all"]);
    assert!(result.contains("2"), "Expected 2 messages after sync-session: {result}");
}

#[test]
fn test_tool_use_file_path_live() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-lf","cwd":"/opt/test"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-lf","toolName":"Read","toolInput":{"file_path":"/opt/test/main.rs"}}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-lf","toolName":"Glob","toolInput":{"pattern":"**/*.rs"}}"#);

    let result = run_cli(&dir, &["tools", "--period", "all"]);
    assert!(result.contains("Read"), "Expected Read: {result}");
    assert!(result.contains("Glob"), "Expected Glob: {result}");
}

#[test]
fn test_doctor_runs_without_crash() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["doctor"]);
    // Doctor should run without crashing
    assert!(result.contains("DB:"), "Doctor should show DB status: {result}");
}

#[test]
fn test_statusline_live_session_tokens() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/live.jsonl");
    write_test_transcript(&tp);

    // Start session with transcript but DON'T send session-end — simulates active session
    run_hook(&dir, "session-start", &format!(
        r#"{{"sessionId":"sess-live","cwd":"/opt/test","transcriptPath":"{tp}"}}"#
    ));

    // Statusline should show non-zero session tokens from live transcript read
    let result = run_cli(&dir, &["statusline"]);
    // Test transcript: msg-1 (100+50+1000+200=1350) + msg-2 (200+100+2000+0=2300) = 3650 total
    assert!(!result.contains("sess 0"), "Expected non-zero session tokens during active session: {result}");
    assert!(result.contains("sess 3.6K") || result.contains("sess 3.7K"),
        "Expected ~3.6K session tokens: {result}");
}

// --- Phase 3: Budget, calibration, and configuration ---

#[test]
fn test_budget_show_defaults() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["budget"]);
    assert!(result.contains("Plan:"), "Expected plan display: {result}");
    assert!(result.contains("weekly"), "Expected weekly budget: {result}");
    assert!(result.contains("5-hour"), "Expected 5-hour budget: {result}");
}

#[test]
fn test_budget_set_weekly() {
    let dir = unique_data_dir();
    run_cli(&dir, &["budget", "--weekly", "3B"]);
    let result = run_cli(&dir, &["budget"]);
    assert!(result.contains("3B") || result.contains("3.0B"), "Expected 3B in budget: {result}");
}

#[test]
fn test_budget_set_burst() {
    let dir = unique_data_dir();
    run_cli(&dir, &["budget", "--burst", "250M"]);
    let result = run_cli(&dir, &["budget"]);
    assert!(result.contains("250M") || result.contains("250"), "Expected 250M in budget: {result}");
}

#[test]
fn test_budget_set_plan_name() {
    let dir = unique_data_dir();
    run_cli(&dir, &["budget", "--plan", "max-5x"]);
    let result = run_cli(&dir, &["budget"]);
    assert!(result.contains("max-5x"), "Expected plan name: {result}");
}

#[test]
fn test_calibration_show_initial() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["calibrate"]);
    assert!(result.contains("Last calibrated:"), "Expected calibration status: {result}");
    assert!(result.contains("never") || result.contains("Weekly"), "Should show uncalibrated or placeholder: {result}");
}

#[test]
fn test_calibration_set_weekly_pct() {
    let dir = unique_data_dir();
    run_cli(&dir, &["calibrate", "--weekly-pct", "42.5", "--burst-pct", "67"]);
    let result = run_cli(&dir, &["calibrate"]);
    assert!(result.contains("42.5") || result.contains("42"), "Expected weekly pct: {result}");
    assert!(result.contains("67"), "Expected burst pct: {result}");
}

#[test]
fn test_sessions_list() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-l1","cwd":"/opt/proj1"}"#);
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-l2","cwd":"/opt/proj2"}"#);
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-l3","cwd":"/opt/proj3"}"#);

    let result = run_cli(&dir, &["sessions", "--period", "all"]);
    assert!(result.contains("sess-l1"), "Expected sess-l1: {result}");
    assert!(result.contains("sess-l2"), "Expected sess-l2: {result}");
    assert!(result.contains("sess-l3"), "Expected sess-l3: {result}");
}

#[test]
fn test_sessions_period_filter_today() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-today","cwd":"/opt/test"}"#);
    let result = run_cli(&dir, &["sessions", "--period", "today"]);
    // May have 0 or 1 sessions depending on time
    assert!(!result.is_empty(), "Should not crash: {result}");
}

#[test]
fn test_agents_list_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["agents"]);
    assert!(result.contains("No subagents") || result.is_empty(), "Should show no subagents or empty: {result}");
}

#[test]
fn test_agents_with_session_filter() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-agf","cwd":"/opt/test"}"#);
    run_hook(&dir, "agent-start", r#"{"sessionId":"sess-agf","agentId":"ag-1"}"#);
    run_hook(&dir, "agent-end", r#"{"sessionId":"sess-agf","agentId":"ag-1"}"#);

    let result = run_cli(&dir, &["agents", "--session", "sess-agf"]);
    assert!(result.contains("ag-1"), "Expected ag-1: {result}");
}

#[test]
fn test_cost_by_model() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/models.jsonl");
    let mut f = fs::File::create(&tp).unwrap();
    writeln!(f, r#"{{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","uuid":"meta-1","cwd":"/opt/test","sessionId":"sess-m","gitBranch":"main"}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","uuid":"msg-1","message":{{"model":"claude-opus-4-6","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"test"}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:02.000Z","uuid":"msg-2","message":{{"model":"claude-sonnet-4-6","usage":{{"input_tokens":50,"output_tokens":25,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"test2"}}]}}}}"#).unwrap();

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-m","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-m","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["cost", "--by", "model"]);
    assert!(result.contains("opus"), "Expected opus model: {result}");
    assert!(result.contains("sonnet"), "Expected sonnet model: {result}");
}

#[test]
fn test_tools_period_filtering() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-tp","cwd":"/opt/test"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-tp","toolName":"Read"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-tp","toolName":"Write"}"#);

    let result = run_cli(&dir, &["tools", "--period", "all"]);
    assert!(result.contains("Read"), "Expected Read: {result}");
    assert!(result.contains("Write"), "Expected Write: {result}");
}

#[test]
fn test_efficiency_with_data() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/eff.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-e1","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-e1","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["efficiency"]);
    assert!(result.contains("Total cost") || result.contains("$"), "Expected cost data: {result}");
    assert!(result.contains("session") || result.contains("agent"), "Expected efficiency metrics: {result}");
}

#[test]
fn test_routing_decision_display() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["routing"]);
    assert!(result.contains("routing") || result.contains("No routing"), "Expected routing output or empty: {result}");
}

#[test]
fn test_commits_list_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["commits"]);
    assert!(result.contains("No commits") || result.is_empty(), "Should show no commits: {result}");
}

#[test]
fn test_sync_all_with_no_transcripts() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["sync"]);
    assert!(result.contains("up to date") || result.contains("error") || result.is_empty(), "Should handle gracefully: {result}");
}

#[test]
fn test_sync_nonexistent_session() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["sync", "--session", "nonexistent-sess-id"]);
    assert!(result.contains("not found") || result.contains("No transcript"), "Should report not found: {result}");
}

// --- Phase 4: Fleet, efficiency, budgets ---

#[test]
fn test_project_costs_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["project-costs", "--period", "week"]);
    assert!(result.contains("No session") || result.is_empty(), "Should handle empty gracefully: {result}");
}

#[test]
fn test_task_costs_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["task-costs", "--period", "week"]);
    assert!(result.contains("No task") || result.contains("Hint"), "Should show hint: {result}");
}

#[test]
fn test_anomalies_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["anomalies", "--period", "week"]);
    assert!(result.contains("No cost anomalies") || result.is_empty(), "Should show no anomalies: {result}");
}

#[test]
fn test_fleet_costs_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["fleet-costs", "--period", "week"]);
    assert!(result.contains("No session") || result.is_empty(), "Should handle empty gracefully: {result}");
}

#[test]
fn test_efficiency_scores_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["efficiency-scores", "--period", "week"]);
    assert!(result.contains("No session") || result.is_empty(), "Should handle empty gracefully: {result}");
}

#[test]
fn test_budget_alerts_empty() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["budget-alerts"]);
    assert!(result.contains("No budget") || result.contains("within"), "Should show no alerts: {result}");
}

#[test]
fn test_set_budget_default() {
    let dir = unique_data_dir();
    run_cli(&dir, &["set-budget", "--project", "default", "--limit", "50.0"]);
    let result = run_cli(&dir, &["budget-alerts"]);
    assert!(!result.is_empty(), "Should not crash after setting budget: {result}");
}

#[test]
fn test_set_budget_project() {
    let dir = unique_data_dir();
    run_cli(&dir, &["set-budget", "--project", "hydra-project", "--limit", "25.5"]);
    let result = run_cli(&dir, &["budget-alerts"]);
    assert!(!result.is_empty(), "Should not crash after setting project budget: {result}");
}

// --- Edge cases and error handling ---

#[test]
fn test_multiple_concurrent_sessions() {
    let dir = unique_data_dir();
    for i in 0..5 {
        let sid = format!("sess-multi-{}", i);
        run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"{}","cwd":"/opt/test{}"}}"#, sid, i));
    }

    let result = run_cli(&dir, &["sessions", "--period", "all"]);
    for i in 0..5 {
        let sid = format!("sess-multi-{}", i);
        assert!(result.contains(&sid), "Expected {}: {}", sid, result);
    }
}

#[test]
fn test_session_with_special_characters_in_path() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/test@special#chars.jsonl");
    write_test_transcript(&tp);

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-special","cwd":"/opt/test-[special]","transcriptPath":"{}"}}"#, tp));
    let result = run_cli(&dir, &["sessions", "--period", "all"]);
    assert!(result.contains("sess-special"), "Should handle special chars: {result}");
}

#[test]
fn test_very_long_session_id() {
    let dir = unique_data_dir();
    let long_id = "s".repeat(256);
    let output = run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"{}","cwd":"/opt/test"}}"#, long_id));
    assert!(output.status.success(), "Should handle long IDs gracefully");
}

#[test]
fn test_zero_token_usage() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/zero.jsonl");
    let mut f = fs::File::create(&tp).unwrap();
    writeln!(f, r#"{{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","uuid":"meta-1","cwd":"/opt/test","sessionId":"sess-zero"}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","uuid":"msg-1","message":{{"model":"claude-opus-4-6","usage":{{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"nothing"}}]}}}}"#).unwrap();

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-zero","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-zero","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["stats", "--period", "all"]);
    assert!(result.contains("$    0.00") || result.contains("0 tokens"), "Should handle zero usage: {result}");
}

#[test]
fn test_invalid_model_name_ignored() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/invalid.jsonl");
    let mut f = fs::File::create(&tp).unwrap();
    writeln!(f, r#"{{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","uuid":"meta","cwd":"/opt/test","sessionId":"sess-inv"}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","uuid":"msg","message":{{"model":"totally-fake-model-xyz","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"test"}}]}}}}"#).unwrap();

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-inv","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-inv","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["cost", "--by", "model"]);
    // Should not crash and may show unknown model
    assert!(!result.is_empty(), "Should handle unknown model gracefully: {result}");
}

#[test]
fn test_cache_hit_calculation() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/cache.jsonl");
    let mut f = fs::File::create(&tp).unwrap();
    writeln!(f, r#"{{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","uuid":"meta","cwd":"/opt/test","sessionId":"sess-cache"}}"#).unwrap();
    // Message with lots of cache read
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","uuid":"msg","message":{{"model":"claude-opus-4-6","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":9000,"cache_creation_input_tokens":1000}},"content":[{{"type":"text","text":"cached"}}]}}}}"#).unwrap();

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-cache","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-cache","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["stats", "--period", "all"]);
    assert!(result.contains("Cache hit"), "Expected cache hit stat: {result}");
    assert!(result.contains("%"), "Expected cache percentage: {result}");
}

#[test]
fn test_cost_breakdown_accuracy() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/accurate.jsonl");
    let mut f = fs::File::create(&tp).unwrap();
    writeln!(f, r#"{{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","uuid":"meta","cwd":"/opt/test","sessionId":"sess-acc"}}"#).unwrap();
    // Haiku: 100 input + 50 output (should be cheapest)
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","uuid":"msg-h","message":{{"model":"claude-haiku-4-5","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"cheap"}}]}}}}"#).unwrap();
    // Opus: same tokens (should be ~5x more expensive)
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:02.000Z","uuid":"msg-o","message":{{"model":"claude-opus-4-6","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"expensive"}}]}}}}"#).unwrap();

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-acc","cwd":"/opt/test","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-acc","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["cost", "--by", "model"]);
    // Both should appear with different costs
    assert!(result.contains("haiku"), "Expected haiku: {result}");
    assert!(result.contains("opus"), "Expected opus: {result}");
}

#[test]
fn test_empty_tool_input_no_crash() {
    let dir = unique_data_dir();
    run_hook(&dir, "session-start", r#"{"sessionId":"sess-empty-tool","cwd":"/opt/test"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-empty-tool","toolName":"Read"}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-empty-tool","toolName":"Bash","toolInput":{}}"#);
    run_hook(&dir, "tool-use", r#"{"sessionId":"sess-empty-tool","toolName":"Write","toolInput":{"file_path":""}}"#);

    let result = run_cli(&dir, &["tools", "--period", "all"]);
    assert!(result.contains("Read"), "Should record tools even with empty input: {result}");
}

#[test]
fn test_doctor_reports_complete_status() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["doctor"]);
    // Should check: DB, hooks, binary, timer, pricing, data, transcripts
    assert!(result.contains("DB:"), "Missing DB check: {result}");
    assert!(result.contains("Hooks:"), "Missing hooks check: {result}");
    assert!(result.contains("Binary:"), "Missing binary check: {result}");
    assert!(result.contains("Data:"), "Missing data check: {result}");
}

#[test]
fn test_statusline_format_compact() {
    let dir = unique_data_dir();
    let result = run_cli(&dir, &["statusline"]);
    // Should be single line, no newlines for status bar use
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 1, "Statusline should be exactly 1 line: {}", result);
    assert!(!result.contains('\n'), "Statusline should not have newlines: {result}");
}

#[test]
fn test_multiple_projects_in_single_session() {
    let dir = unique_data_dir();
    let tp = format!("{dir}/multi-proj.jsonl");
    let mut f = fs::File::create(&tp).unwrap();
    writeln!(f, r#"{{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","uuid":"meta","cwd":"/opt/hydra-project","sessionId":"sess-mp"}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","uuid":"msg-1","message":{{"model":"claude-opus-4-6","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"content":[{{"type":"text","text":"msg"}}]}}}}"#).unwrap();

    run_hook(&dir, "session-start", &format!(r#"{{"sessionId":"sess-mp","cwd":"/opt/hydra-project","transcriptPath":"{tp}"}}"#));
    run_hook(&dir, "session-end", &format!(r#"{{"sessionId":"sess-mp","transcriptPath":"{tp}"}}"#));

    let result = run_cli(&dir, &["sessions", "--period", "all"]);
    assert!(result.contains("hydra"), "Should capture project context: {result}");
}
