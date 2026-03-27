//! costguard-pulse — Claude Code session analytics CLI
//!
//! Track tokens, costs, and efficiency across Claude Code sessions.

use anyhow::Result;
use chrono::{Duration, Utc};
use clap::{Parser, Subcommand};
use colored::Colorize;
use rusqlite::Connection;

mod db;
mod routing;
mod commits;
mod transcript;

#[derive(Parser)]
#[command(name = "costguard-pulse", about = "Claude Code session analytics — where your tokens go")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show usage statistics
    Stats {
        #[arg(short, long, default_value = "today")]
        period: String,
    },
    /// Show cost breakdown
    Cost {
        #[arg(short, long, default_value = "session")]
        by: String,
    },
    /// List sessions
    Sessions {
        #[arg(short, long, default_value = "today")]
        period: String,
    },
    /// Show subagent breakdown
    Agents {
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Show tool usage
    Tools {
        #[arg(short, long, default_value = "today")]
        period: String,
    },
    /// Show efficiency metrics
    Efficiency,
    /// Print status line for Claude Code terminal
    Statusline,
    /// Show model routing decisions
    Routing,
    /// Show git commits linked to sessions
    Commits {
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Import commits from a project into current session
    ImportCommits {
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        session: String,
    },
    /// Parse transcript files to backfill token/cost/tool data
    Sync {
        /// Specific session ID to sync (default: all unsynced)
        #[arg(short, long)]
        session: Option<String>,
        /// Force re-sync even if session already has data
        #[arg(short, long)]
        force: bool,
    },
    /// Set or show usage budget (weekly + 5-hour rolling windows)
    Budget {
        /// Set weekly token cap (e.g., 4B, 5B)
        #[arg(short, long)]
        weekly: Option<String>,
        /// Set 5-hour burst token cap (e.g., 500M, 1B)
        #[arg(short = 'b', long = "burst")]
        five_hr: Option<String>,
        /// Set plan name (e.g., max, max-5x)
        #[arg(short, long)]
        plan: Option<String>,
    },
    /// Calibrate budget percentages against claude.ai/settings/usage
    Calibrate {
        /// Actual weekly percentage from usage page (e.g., 33)
        #[arg(short, long)]
        weekly_pct: Option<f64>,
        /// Actual 5-hour burst percentage from usage page (e.g., 65)
        #[arg(short = 'b', long = "burst-pct")]
        burst_pct: Option<f64>,
    },
    /// Sync transcripts from a remote host (cross-machine analytics)
    SyncRemote {
        /// Remote host to sync from (e.g., myserver.local)
        #[arg(short = 'H', long)]
        host: String,
        /// SSH user
        #[arg(short, long, default_value = "user")]
        user: String,
    },
    /// Health check
    Doctor {
        /// Show recent sync errors
        #[arg(long)]
        errors: bool,
    },
    /// Install systemd user timer for periodic background sync
    InstallTimer,
    /// Show per-project cost breakdown
    ProjectCosts {
        #[arg(short, long, default_value = "week")]
        period: String,
    },
    /// Show per-task cost breakdown
    TaskCosts {
        #[arg(short, long, default_value = "week")]
        period: String,
    },
    /// Detect cost anomalies (sessions >2 std devs from mean)
    Anomalies {
        #[arg(short, long, default_value = "week")]
        period: String,
    },
}

/// Parse human-friendly token amounts: "500M", "1B", "2.5B", "1000000000"
fn parse_token_amount(s: &str) -> i64 {
    let s = s.trim().to_uppercase();
    if let Some(n) = s.strip_suffix('B') {
        n.parse::<f64>().map(|v| (v * 1_000_000_000.0) as i64).unwrap_or(0)
    } else if let Some(n) = s.strip_suffix('M') {
        n.parse::<f64>().map(|v| (v * 1_000_000.0) as i64).unwrap_or(0)
    } else if let Some(n) = s.strip_suffix('K') {
        n.parse::<f64>().map(|v| (v * 1_000.0) as i64).unwrap_or(0)
    } else {
        s.parse::<i64>().unwrap_or(0)
    }
}

fn period_to_timestamp(period: &str) -> String {
    match period {
        "today" => Utc::now().format("%Y-%m-%d").to_string(),
        "week" => (Utc::now() - Duration::days(7)).to_rfc3339(),
        "month" => (Utc::now() - Duration::days(30)).to_rfc3339(),
        _ => "2020-01-01".to_string(), // "all"
    }
}

fn period_filter(period: &str) -> String {
    match period {
        "today" => {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            format!("started_at >= '{today}'")
        }
        "week" => {
            let week_ago = (Utc::now() - Duration::days(7)).format("%Y-%m-%dT%H:%M:%S").to_string();
            format!("started_at >= '{week_ago}'")
        }
        "month" => {
            let month_ago = (Utc::now() - Duration::days(30)).format("%Y-%m-%dT%H:%M:%S").to_string();
            format!("started_at >= '{month_ago}'")
        }
        _ => "1=1".to_string(),
    }
}

fn format_tokens(n: i64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

fn budget_bar(label: &str, pct: f64, used: i64, cap: i64) -> String {
    let bar_width = 25;
    let filled = ((pct / 100.0) * bar_width as f64).min(bar_width as f64).max(0.0) as usize;
    let empty = bar_width - filled;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
    let colored_bar = if pct > 80.0 { bar.red().to_string() }
        else if pct > 50.0 { bar.yellow().to_string() }
        else { bar.green().to_string() };
    format!("  {:>8} [{colored_bar}] {:>5.1}%  {} / {}",
        label.bold(), pct, format_tokens(used), format_tokens(cap))
}

fn cmd_stats(conn: &Connection, period: &str) -> Result<()> {
    let filter = period_filter(period);
    let budget = db::get_plan_budget(conn);
    let now = Utc::now();

    let (sessions, input, output, cache_read, cache_write, cost): (i64, i64, i64, i64, i64, f64) = conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(total_input_tokens),0), COALESCE(SUM(total_output_tokens),0), COALESCE(SUM(total_cache_read),0), COALESCE(SUM(total_cache_write),0), COALESCE(SUM(total_cost_usd),0) FROM sessions WHERE {filter}"
        ),
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;

    let messages: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM messages m JOIN sessions s ON m.session_id = s.id WHERE {filter}"),
        [], |row| row.get(0),
    ).unwrap_or(0);

    let total_in = input + cache_read + cache_write;
    let total_all = total_in + output;
    let hit_rate = if total_in > 0 { cache_read as f64 / total_in as f64 * 100.0 } else { 0.0 };

    // Rolling window calculations
    let week_start = (now - Duration::days(7)).format("%Y-%m-%dT%H:%M:%S").to_string();
    let five_hr_start = (now - Duration::hours(5)).format("%Y-%m-%dT%H:%M:%S").to_string();

    let week_tokens = db::tokens_in_window(conn, &week_start);
    let five_hr_tokens = db::tokens_in_window(conn, &five_hr_start);

    let week_pct = week_tokens as f64 / budget.weekly_tokens as f64 * 100.0;
    let five_hr_pct = five_hr_tokens as f64 / budget.five_hr_tokens as f64 * 100.0;

    // Daily burn rate (over last 7 days)
    let daily_rate = week_tokens / 7;

    println!("{}", format!("  costguard-pulse stats — {period}").cyan().bold());
    println!("  {}", "─".repeat(52).dimmed());

    // Budget bars
    println!("{}", format!("  {} plan", budget.plan_name).bold());
    println!("{}", budget_bar("weekly", week_pct, week_tokens, budget.weekly_tokens));
    println!("{}", budget_bar("5-hour", five_hr_pct, five_hr_tokens, budget.five_hr_tokens));
    println!("    {} avg/day  |  {:.1} days at current rate",
        format_tokens(daily_rate),
        if daily_rate > 0 { (budget.weekly_tokens - week_tokens) as f64 / daily_rate as f64 } else { 99.0 });
    println!();

    // Period stats
    println!("  {} {sessions}  {} {messages}", "Sessions".bold(), "Messages".bold());
    println!("  {} {:>12}  (period total)", "Tokens".bold(), format_tokens(total_all));
    println!("    {}", format!("input:        {}", format_tokens(input)).dimmed());
    println!("    {}", format!("cache write:  {}", format_tokens(cache_write)).dimmed());
    println!("    {}", format!("cache read:   {}", format_tokens(cache_read)).green());
    println!("    {}", format!("output:       {}", format_tokens(output)).dimmed());
    println!("  {} {:>11}", "Cache hit".bold(), format!("{hit_rate:.1}%").green());
    println!("  {} {:>11}", "Est. cost".bold(), format!("${cost:.2}").dimmed());

    // Top tools
    println!();
    println!("  {}", "Top tools".bold());
    let mut stmt = conn.prepare(&format!(
        "SELECT t.tool_name, COUNT(*) FROM tool_usage t JOIN sessions s ON t.session_id = s.id WHERE {filter} GROUP BY t.tool_name ORDER BY COUNT(*) DESC LIMIT 10"
    ))?;
    let tools: Vec<(String, i64)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.filter_map(|r| r.ok()).collect();
    let max_count = tools.first().map(|t| t.1).unwrap_or(1);
    for (name, count) in &tools {
        let bar_len = (*count as f64 / max_count as f64 * 20.0) as usize;
        let bar = "█".repeat(bar_len);
        println!("    {:20} {:>5}  {}", name.cyan(), count, bar.cyan());
    }
    if tools.is_empty() {
        println!("    {}", "(none — run `costguard-pulse sync --force` to extract from transcripts)".dimmed());
    }

    // Subagent summary
    let agent_count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM subagents a JOIN sessions s ON a.session_id = s.id WHERE {filter}"),
        [], |row| row.get(0),
    ).unwrap_or(0);
    if agent_count > 0 {
        let agent_cost: f64 = conn.query_row(
            &format!("SELECT COALESCE(SUM(a.cost_usd),0) FROM subagents a JOIN sessions s ON a.session_id = s.id WHERE {filter}"),
            [], |row| row.get(0),
        ).unwrap_or(0.0);
        println!();
        println!("  {}", "Subagents".bold());
        println!("    Spawned: {agent_count}  Cost: ${agent_cost:.2}");
    }

    Ok(())
}

fn cmd_cost(conn: &Connection, by: &str) -> Result<()> {
    println!("{}", "  costguard-pulse cost".cyan().bold());
    println!("  {}", "─".repeat(40).dimmed());

    match by {
        "session" => {
            let mut stmt = conn.prepare(
                "SELECT id, hostname, COALESCE(project,''), COALESCE(total_cost_usd,0), COALESCE(total_output_tokens,0), started_at FROM sessions ORDER BY total_cost_usd DESC LIMIT 20"
            )?;
            let rows: Vec<(String, String, String, f64, i64, String)> = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
            })?.filter_map(|r| r.ok()).collect();
            for (sid, host, proj, cost, out, date) in &rows {
                let short = &sid[..sid.len().min(12)];
                let short_proj = proj.split('/').last().unwrap_or(proj);
                println!("    {} {:>10} {:>15} ${cost:>8.2}  {} {}", short.cyan(), host, short_proj, format_tokens(*out).dimmed(), date[..10].to_string().dimmed());
            }
        }
        "model" => {
            let mut stmt = conn.prepare(
                "SELECT model, COUNT(*), SUM(cost_usd) FROM messages WHERE model IS NOT NULL AND model != '' GROUP BY model ORDER BY SUM(cost_usd) DESC"
            )?;
            let rows: Vec<(String, i64, f64)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.filter_map(|r| r.ok()).collect();
            for (model, count, cost) in &rows {
                println!("    {:40} {:>6} msgs  ${cost:.2}", model, count);
            }
        }
        "subagent" => {
            let mut stmt = conn.prepare(
                "SELECT model, COUNT(*), COALESCE(SUM(cost_usd),0) FROM subagents GROUP BY model ORDER BY SUM(cost_usd) DESC"
            )?;
            let rows: Vec<(String, i64, f64)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.filter_map(|r| r.ok()).collect();
            for (model, count, cost) in &rows {
                println!("    {:40} {:>4} agents  ${cost:.2}", model, count);
            }
        }
        _ => println!("  Use: session, model, subagent"),
    }

    Ok(())
}

fn cmd_sessions(conn: &Connection, period: &str) -> Result<()> {
    let filter = period_filter(period);
    println!("{}", format!("  costguard-pulse sessions — {period}").cyan().bold());
    println!("  {}", "─".repeat(60).dimmed());

    let mut stmt = conn.prepare(&format!(
        "SELECT id, hostname, COALESCE(project,''), COALESCE(model,''), started_at, COALESCE(total_cost_usd,0), COALESCE(total_output_tokens,0) FROM sessions WHERE {filter} ORDER BY started_at DESC"
    ))?;
    let rows: Vec<(String, String, String, String, String, f64, i64)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
    })?.filter_map(|r| r.ok()).collect();

    for (sid, host, proj, _model, start, cost, _out) in &rows {
        let short = &sid[..sid.len().min(12)];
        let short_proj = proj.split('/').last().unwrap_or(proj);
        println!("    {} {:>10} {:>15}  ${cost:>7.2}  {}", short.cyan(), host, short_proj, start[..16].to_string().dimmed());
    }

    Ok(())
}

fn cmd_agents(conn: &Connection, session: &Option<String>) -> Result<()> {
    println!("{}", "  costguard-pulse subagents".cyan().bold());
    println!("  {}", "─".repeat(60).dimmed());

    let query = match session {
        Some(sid) => format!("SELECT id, COALESCE(model,''), started_at, COALESCE(input_tokens,0), COALESCE(output_tokens,0), COALESCE(cost_usd,0), COALESCE(task_description,'') FROM subagents WHERE session_id LIKE '{}%' ORDER BY started_at", sid),
        None => "SELECT id, COALESCE(model,''), started_at, COALESCE(input_tokens,0), COALESCE(output_tokens,0), COALESCE(cost_usd,0), COALESCE(task_description,'') FROM subagents ORDER BY started_at DESC LIMIT 30".to_string(),
    };

    let mut stmt = conn.prepare(&query)?;
    let rows: Vec<(String, String, String, i64, i64, f64, String)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
    })?.filter_map(|r| r.ok()).collect();

    for (id, model, _start, inp, out, cost, desc) in &rows {
        let short = &id[..id.len().min(12)];
        let short_model = model.split('-').last().unwrap_or(model);
        println!("    {} {:>8} in={:>8} out={:>6} ${cost:.3}  {}", short.dimmed(), short_model.cyan(), format_tokens(*inp), format_tokens(*out), &desc[..desc.len().min(50)]);
    }

    if rows.is_empty() {
        println!("    No subagents recorded.");
    }

    Ok(())
}

/// Snapshot-delta calibration model.
///
/// At calibration time we store: actual% (from claude.ai) + our tracked tokens at that moment.
/// Between calibrations, estimate: cal_actual + (current_tracked - tracked_at_cal) / budget * 100
///
/// 5h calibration expires after 5h (cal point rolls out of window).
/// Weekly calibration expires after 24h.
/// Returns (week_est, 5h_est, staleness_minutes, is_calibrated).
fn calibrated_pcts(conn: &Connection, tracked_week_pct: f64, tracked_5h_pct: f64,
                   week_tokens: i64, five_hr_tokens: i64, budget: &db::PlanBudget) -> (f64, f64, i64, bool) {
    let cal_ts = db::get_config(conn, "calibration_timestamp");
    let now = Utc::now();

    let cal_time = cal_ts.as_ref().and_then(|ts|
        chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S").ok()
    );

    let staleness_mins = cal_time.map(|ct|
        (now.naive_utc() - ct).num_minutes()
    ).unwrap_or(9999);

    if cal_time.is_none() {
        return (tracked_week_pct, tracked_5h_pct, staleness_mins, false);
    }

    // 5h estimate: snapshot-delta if calibration is <5h old
    let five_hr_est = if staleness_mins < 300 {
        let cal_actual = db::get_config(conn, "cal_actual_5h_pct")
            .and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let cal_tracked = db::get_config(conn, "cal_tracked_5h_tokens")
            .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let tracked_delta = (five_hr_tokens - cal_tracked).max(0);
        let delta_pct = tracked_delta as f64 / budget.five_hr_tokens as f64 * 100.0;
        (cal_actual + delta_pct).clamp(0.0, 100.0)
    } else {
        // Calibration rolled out of 5h window — tracked only
        tracked_5h_pct
    };

    // Weekly estimate: snapshot-delta if calibration is <24h old
    let week_est = if staleness_mins < 1440 {
        let cal_actual = db::get_config(conn, "cal_actual_weekly_pct")
            .and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let cal_tracked = db::get_config(conn, "cal_tracked_weekly_tokens")
            .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let tracked_delta = (week_tokens - cal_tracked).max(0);
        let delta_pct = tracked_delta as f64 / budget.weekly_tokens as f64 * 100.0;
        (cal_actual + delta_pct).clamp(0.0, 100.0)
    } else {
        tracked_week_pct
    };

    (week_est, five_hr_est, staleness_mins, true)
}

fn cmd_statusline(conn: &Connection) -> Result<()> {
    let budget = db::get_plan_budget(conn);
    let now = Utc::now();

    let week_start = (now - Duration::days(7)).format("%Y-%m-%dT%H:%M:%S").to_string();
    let five_hr_start = (now - Duration::hours(5)).format("%Y-%m-%dT%H:%M:%S").to_string();

    let mut week_tokens = db::tokens_in_window(conn, &week_start);
    let mut five_hr_tokens = db::tokens_in_window(conn, &five_hr_start);
    let cache_hit = db::cache_hit_in_window(conn, &week_start);

    // Get current session token usage (most recent session)
    let (db_session_tokens, transcript_path, session_started): (i64, Option<String>, Option<String>) = conn.query_row(
        "SELECT COALESCE(total_input_tokens + total_output_tokens + total_cache_read + total_cache_write, 0), transcript_path, started_at FROM sessions ORDER BY started_at DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap_or((0, None, None));

    // Always try live transcript for the most recent session — DB may have stale/partial data.
    // Use the larger of DB vs live value (live is always more current for active sessions).
    let live_tokens = transcript_path
        .as_deref()
        .filter(|tp| !tp.is_empty() && std::path::Path::new(tp).exists())
        .and_then(|tp| transcript::quick_sum_tokens(tp).ok())
        .map(|(i, o, cr, cw)| i + o + cr + cw)
        .unwrap_or(0);

    let session_tokens = db_session_tokens.max(live_tokens);

    // If live tokens > DB tokens, the window aggregates are understated by the delta.
    // Add the live delta to any window the current session falls within.
    let live_delta = (live_tokens - db_session_tokens).max(0);
    if live_delta > 0 {
        if let Some(ref started) = session_started {
            if started.as_str() >= week_start.as_str() {
                week_tokens += live_delta;
            }
            if started.as_str() >= five_hr_start.as_str() {
                five_hr_tokens += live_delta;
            }
        }
    }

    let week_pct = week_tokens as f64 / budget.weekly_tokens as f64 * 100.0;
    let five_hr_pct = five_hr_tokens as f64 / budget.five_hr_tokens as f64 * 100.0;

    let (wk, fh, stale_mins, _is_cal) = calibrated_pcts(conn, week_pct, five_hr_pct,
        week_tokens, five_hr_tokens, &budget);

    // Staleness indicator for 5h window (only when calibrated)
    let stale_tag = if stale_mins >= 9999 {
        String::new()
    } else if stale_mins < 5 {
        " ~".to_string()
    } else if stale_mins < 60 {
        format!(" ~{}m", stale_mins)
    } else if stale_mins < 300 {
        format!(" ~{}h{}m", stale_mins / 60, stale_mins % 60)
    } else {
        " ??".to_string()
    };

    print!(
        "wk {wk:.0}% ({}) | 5h {fh:.0}%{stale_tag} | sess {} | {:.0}% cache",
        format_tokens(week_tokens),
        format_tokens(session_tokens),
        cache_hit,
    );
    Ok(())
}

fn cmd_doctor(conn: &Connection, show_errors: bool) -> Result<()> {
    println!("{}", "  costguard-pulse doctor".cyan().bold());
    println!("  {}", "─".repeat(40).dimmed());

    // DB check
    let path = db::db_path();
    println!("  DB: {} {}", if path.exists() { "✓".green() } else { "✗".red() }, path.display());

    // Hook config check
    let settings_path = dirs::home_dir().unwrap_or_default().join(".claude/settings.json");
    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path).unwrap_or_default();
        let expected_hooks = ["session-start", "session-end", "tool-use", "agent-start", "agent-end"];
        let mut missing = Vec::new();
        for hook in &expected_hooks {
            if !content.contains(&format!("costguard-pulse-hook {hook}")) {
                missing.push(*hook);
            }
        }
        if missing.is_empty() {
            println!("  Hooks: {} all 5 installed", "✓".green());
        } else {
            println!("  Hooks: {} missing: {}", "⚠".yellow(), missing.join(", "));
        }
    } else {
        println!("  Hooks: {} settings.json not found", "✗".red());
    }

    // Binary check
    let bin_path = dirs::home_dir().unwrap_or_default().join(".local/bin/costguard-pulse-hook");
    println!("  Binary: {} {}", if bin_path.exists() { "✓".green() } else { "✗".red() }, bin_path.display());

    // Timer check
    let timer_active = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "costguard-pulse-sync.timer"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    println!("  Timer: {} {}", if timer_active { "✓".green() } else { "⚠".yellow() },
        if timer_active { "active (15min sync)" } else { "not installed (run `costguard-pulse install-timer`)" });

    // Pricing table check
    let pricing_count: i64 = conn.query_row("SELECT COUNT(*) FROM model_pricing", [], |r| r.get(0))?;
    let known_models: Vec<String> = {
        let mut stmt = conn.prepare("SELECT DISTINCT model FROM messages WHERE model IS NOT NULL AND model != '' AND model NOT LIKE '<%'")?;
        stmt.query_map([], |row| row.get(0))?.filter_map(|r| r.ok()).collect()
    };
    let unpriced: Vec<&String> = known_models.iter().filter(|m| {
        conn.query_row("SELECT COUNT(*) FROM model_pricing WHERE model = ?1", [m.as_str()], |r| r.get::<_, i64>(0)).unwrap_or(0) == 0
    }).collect();
    if unpriced.is_empty() {
        println!("  Pricing: {} {pricing_count} models", "✓".green());
    } else {
        println!("  Pricing: {} {pricing_count} models, missing: {}", "⚠".yellow(), unpriced.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
    }

    // Data summary
    let sessions: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
    let messages: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
    let tools: i64 = conn.query_row("SELECT COUNT(*) FROM tool_usage", [], |r| r.get(0))?;
    let commits_count: i64 = conn.query_row("SELECT COUNT(*) FROM commits", [], |r| r.get(0))?;
    println!("  Data: {sessions} sessions, {messages} messages, {tools} tool calls, {commits_count} commits");

    // Stale sessions (using last_synced_at for detection)
    let stale: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE last_synced_at IS NULL AND transcript_path IS NOT NULL AND transcript_path != ''",
        [], |r| r.get(0),
    )?;
    if stale > 0 {
        println!("  Stale: {} {stale} sessions never synced (run `costguard-pulse sync`)", "⚠".yellow());
    }

    // Sync errors
    let error_count: i64 = conn.query_row("SELECT COUNT(*) FROM sync_errors", [], |r| r.get(0)).unwrap_or(0);
    if error_count > 0 {
        println!("  Sync errors: {} {error_count} total (run `costguard-pulse doctor --errors`)", "⚠".yellow());
    }

    if show_errors && error_count > 0 {
        println!();
        println!("  {}", "Recent sync errors:".bold());
        let mut stmt = conn.prepare("SELECT session_id, error_message, timestamp FROM sync_errors ORDER BY timestamp DESC LIMIT 10")?;
        let errors: Vec<(String, String, String)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?.filter_map(|r| r.ok()).collect();
        for (sid, msg, ts) in &errors {
            let short = &sid[..sid.len().min(12)];
            println!("    {} {} {}", short.dimmed(), msg.red(), ts[..19].to_string().dimmed());
        }
    }

    // Transcript accessibility
    let mut stmt = conn.prepare("SELECT id, transcript_path FROM sessions WHERE transcript_path IS NOT NULL AND transcript_path != '' ORDER BY started_at DESC LIMIT 5")?;
    let recent: Vec<(String, String)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.filter_map(|r| r.ok()).collect();
    let mut inaccessible = 0;
    for (_sid, tp) in &recent {
        if !std::path::Path::new(tp).exists() {
            inaccessible += 1;
        }
    }
    if inaccessible > 0 {
        println!("  Transcripts: {} {inaccessible}/{} recent transcripts inaccessible", "⚠".yellow(), recent.len());
    } else if !recent.is_empty() {
        println!("  Transcripts: {} {}/{} recent accessible", "✓".green(), recent.len(), recent.len());
    }

    Ok(())
}

fn cmd_sync(conn: &Connection, session: Option<String>, force: bool) -> Result<()> {
    println!("{}", "  costguard-pulse sync".cyan().bold());
    println!("  {}", "─".repeat(40).dimmed());

    match session {
        Some(sid) => {
            // Find transcript for specific session
            let path: Option<String> = conn.query_row(
                "SELECT transcript_path FROM sessions WHERE id = ?1",
                [&sid], |row| row.get(0),
            ).ok().flatten();

            let transcript_path = if let Some(p) = path.filter(|p| !p.is_empty()) {
                Some(p)
            } else {
                // Search filesystem
                let claude_dir = dirs::home_dir().unwrap_or_default().join(".claude/projects");
                let mut found = None;
                if let Ok(entries) = std::fs::read_dir(&claude_dir) {
                    for entry in entries.flatten() {
                        let candidate = entry.path().join(format!("{sid}.jsonl"));
                        if candidate.exists() {
                            found = Some(candidate.to_str().unwrap_or("").to_string());
                            break;
                        }
                    }
                }
                found
            };

            match transcript_path {
                Some(p) => {
                    match transcript::parse(conn, &sid, &p) {
                        Ok(r) => println!("  Synced {sid}: {} messages, {} tools, ${:.2}", r.message_count, r.tool_count, r.total_cost),
                        Err(e) => println!("  Error: {e}"),
                    }
                }
                None => println!("  No transcript found for session {sid}"),
            }
        }
        None => {
            match transcript::sync_all(conn, force) {
                Ok(results) => {
                    if results.is_empty() {
                        println!("  All sessions up to date.");
                    } else {
                        for (sid, r) in &results {
                            let short = &sid[..sid.len().min(12)];
                            println!("  {} {} msgs  {} tools  ${:.2}", short.cyan(), r.message_count, r.tool_count, r.total_cost);
                        }
                        println!("  Synced {} sessions.", results.len());
                    }
                }
                Err(e) => println!("  Error: {e}"),
            }
        }
    }

    Ok(())
}

fn cmd_install_timer() -> Result<()> {
    println!("{}", "  costguard-pulse install-timer".cyan().bold());
    println!("  {}", "─".repeat(40).dimmed());

    let home = dirs::home_dir().unwrap_or_default();
    let systemd_dir = home.join(".config/systemd/user");
    std::fs::create_dir_all(&systemd_dir)?;

    let service_path = systemd_dir.join("costguard-pulse-sync.service");
    let timer_path = systemd_dir.join("costguard-pulse-sync.timer");

    let service_content = "[Unit]\nDescription=CostGuard Pulse periodic transcript sync\n\n[Service]\nType=oneshot\nExecStart=%h/.local/bin/costguard-pulse sync\nEnvironment=HOME=%h\n\n[Install]\nWantedBy=default.target\n";

    let timer_content = "[Unit]\nDescription=CostGuard Pulse sync timer\n\n[Timer]\nOnBootSec=5min\nOnUnitActiveSec=15min\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n";

    std::fs::write(&service_path, service_content)?;
    println!("  Wrote {}", service_path.display());

    std::fs::write(&timer_path, timer_content)?;
    println!("  Wrote {}", timer_path.display());

    // Reload and enable
    let reload = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    match reload {
        Ok(s) if s.success() => println!("  Reloaded systemd user daemon"),
        _ => println!("  {} Failed to reload systemd", "⚠".yellow()),
    }

    let enable = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "costguard-pulse-sync.timer"])
        .status();
    match enable {
        Ok(s) if s.success() => println!("  {} Timer enabled and started", "✓".green()),
        _ => println!("  {} Failed to enable timer", "✗".red()),
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let conn = db::open()?;

    match cli.command {
        Commands::Stats { period } => cmd_stats(&conn, &period)?,
        Commands::Cost { by } => cmd_cost(&conn, &by)?,
        Commands::Sessions { period } => cmd_sessions(&conn, &period)?,
        Commands::Agents { session } => cmd_agents(&conn, &session)?,
        Commands::Tools { period } => {
            let filter = period_filter(&period);
            println!("{}", format!("  costguard-pulse tools — {period}").cyan().bold());
            let mut stmt = conn.prepare(&format!(
                "SELECT t.tool_name, COUNT(*) FROM tool_usage t JOIN sessions s ON t.session_id = s.id WHERE {filter} GROUP BY t.tool_name ORDER BY COUNT(*) DESC"
            ))?;
            let rows: Vec<(String, i64)> = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.filter_map(|r| r.ok()).collect();
            let max_count = rows.first().map(|t| t.1).unwrap_or(1);
            for (name, count) in &rows {
                let bar_len = (*count as f64 / max_count as f64 * 20.0) as usize;
                let bar = "█".repeat(bar_len);
                println!("    {:20} {:>5}  {}", name.cyan(), count, bar.cyan());
            }
            if rows.is_empty() {
                println!("    {}", "(none — run `costguard-pulse sync --force` to extract from transcripts)".dimmed());
            }
        }
        Commands::Efficiency => {
            println!("{}", "  costguard-pulse efficiency".cyan().bold());
            println!("  {}", "─".repeat(40).dimmed());
            let total_cost: f64 = conn.query_row("SELECT COALESCE(SUM(total_cost_usd),0) FROM sessions", [], |r| r.get(0))?;
            let total_commits: i64 = conn.query_row("SELECT COUNT(*) FROM commits", [], |r| r.get(0))?;
            let total_agents: i64 = conn.query_row("SELECT COUNT(*) FROM subagents", [], |r| r.get(0))?;
            let agent_cost: f64 = conn.query_row("SELECT COALESCE(SUM(cost_usd),0) FROM subagents", [], |r| r.get(0))?;
            let total_tools: i64 = conn.query_row("SELECT COUNT(*) FROM tool_usage", [], |r| r.get(0))?;
            let total_sessions: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
            println!("  Total cost:        ${total_cost:.2}");
            if total_sessions > 0 { println!("  Avg cost/session:  ${:.2}", total_cost / total_sessions as f64); }
            if total_commits > 0 { println!("  Cost/commit:       ${:.2}", total_cost / total_commits as f64); }
            if total_agents > 0 { println!("  Avg agent cost:    ${:.3}", agent_cost / total_agents as f64); }
            println!("  Agent overhead:    ${agent_cost:.2} ({:.1}%)", if total_cost > 0.0 { agent_cost / total_cost * 100.0 } else { 0.0 });
            if total_sessions > 0 { println!("  Avg tools/session: {:.0}", total_tools as f64 / total_sessions as f64); }
        }
        Commands::Statusline => cmd_statusline(&conn)?,
        Commands::Routing => {
            println!("{}", "  costguard-pulse routing decisions".cyan().bold());
            println!("  {}", "─".repeat(50).dimmed());
            let mut stmt = conn.prepare(
                "SELECT id, model, recommended_model, routing_reason, cost_usd FROM subagents WHERE recommended_model IS NOT NULL AND recommended_model != '' ORDER BY started_at DESC LIMIT 20"
            )?;
            let rows: Vec<(String, String, String, String, f64)> = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?.filter_map(|r| r.ok()).collect();
            for (id, actual, recommended, reason, cost) in &rows {
                let short = &id[..id.len().min(12)];
                let match_status = if actual.contains(recommended) { "✓".green() } else { "✗ OVERRIDE".red() };
                println!("    {} rec={:>8} act={:>8} {} ${cost:.3}  {}", short.dimmed(), recommended, actual, match_status, reason.dimmed());
            }
            if rows.is_empty() {
                println!("    No routing decisions recorded yet.");
            }
        }
        Commands::Commits { session } => {
            println!("{}", "  costguard-pulse commits".cyan().bold());
            let query = match &session {
                Some(sid) => format!("SELECT hash, project, message, files_changed, timestamp FROM commits WHERE session_id LIKE '{}%' ORDER BY timestamp DESC", sid),
                None => "SELECT hash, project, message, files_changed, timestamp FROM commits ORDER BY timestamp DESC LIMIT 30".to_string(),
            };
            let mut stmt = conn.prepare(&query)?;
            let rows: Vec<(String, String, String, i64, String)> = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?.filter_map(|r| r.ok()).collect();
            for (hash, proj, msg, files, ts) in &rows {
                let short_hash = &hash[..hash.len().min(8)];
                let short_proj = proj.split('/').last().unwrap_or(proj);
                println!("    {} {:>15} {:>2} files  {}  {}", short_hash.yellow(), short_proj, files, msg, ts[..10].to_string().dimmed());
            }
            if rows.is_empty() { println!("    No commits linked yet."); }
        }
        Commands::ImportCommits { project, session } => {
            match commits::scan_and_link(&conn, &session, &project) {
                Ok(n) => println!("  Imported {n} commits from {project}"),
                Err(e) => println!("  Error: {e}"),
            }
        }
        Commands::Sync { session, force } => cmd_sync(&conn, session, force)?,
        Commands::Budget { weekly, five_hr, plan } => {
            if weekly.is_none() && five_hr.is_none() && plan.is_none() {
                let budget = db::get_plan_budget(&conn);
                let now = Utc::now();
                let week_start = (now - Duration::days(7)).format("%Y-%m-%dT%H:%M:%S").to_string();
                let five_hr_start = (now - Duration::hours(5)).format("%Y-%m-%dT%H:%M:%S").to_string();
                let week_tokens = db::tokens_in_window(&conn, &week_start);
                let five_hr_tokens = db::tokens_in_window(&conn, &five_hr_start);

                let week_pct = week_tokens as f64 / budget.weekly_tokens as f64 * 100.0;
                let five_hr_pct = five_hr_tokens as f64 / budget.five_hr_tokens as f64 * 100.0;
                let (adj_week, adj_5h, stale_mins, is_cal) = calibrated_pcts(&conn, week_pct, five_hr_pct,
                    week_tokens, five_hr_tokens, &budget);

                println!("{}", "  costguard-pulse budget".cyan().bold());
                println!("  {}", "─".repeat(52).dimmed());
                println!("  Plan:      {}", budget.plan_name);
                if is_cal {
                    println!("{}", budget_bar("weekly", adj_week, (adj_week / 100.0 * budget.weekly_tokens as f64) as i64, budget.weekly_tokens));
                    println!("{}", budget_bar("5-hour", adj_5h, (adj_5h / 100.0 * budget.five_hr_tokens as f64) as i64, budget.five_hr_tokens));
                    let stale_label = if stale_mins < 5 { "just now".to_string() }
                        else if stale_mins < 60 { format!("{}m ago", stale_mins) }
                        else { format!("{}h{}m ago", stale_mins / 60, stale_mins % 60) };
                    println!("  {} calibrated ({}) — tracked {week_pct:.0}%→{adj_week:.0}% wk, {five_hr_pct:.0}%→{adj_5h:.0}% 5h",
                        "✓".green(), stale_label);
                    if stale_mins > 30 {
                        println!("  {} recalibrate: costguard-pulse calibrate --weekly-pct X --burst-pct Y", "⚠".yellow());
                    }
                } else {
                    println!("{}", budget_bar("weekly", week_pct, week_tokens, budget.weekly_tokens));
                    println!("{}", budget_bar("5-hour", five_hr_pct, five_hr_tokens, budget.five_hr_tokens));
                }
                println!();
                println!("  Calibrate from claude.ai/settings/usage:");
                println!("    costguard-pulse calibrate --weekly-pct 33 --burst-pct 65");
            } else {
                if let Some(w) = weekly {
                    let parsed = parse_token_amount(&w);
                    if parsed > 0 {
                        db::set_config(&conn, "weekly_budget_tokens", &parsed.to_string())?;
                        println!("  Weekly cap set to {}", format_tokens(parsed));
                    } else {
                        println!("  Invalid: {w}  (use e.g. 4B, 5B)");
                    }
                }
                if let Some(b) = five_hr {
                    let parsed = parse_token_amount(&b);
                    if parsed > 0 {
                        db::set_config(&conn, "five_hr_budget_tokens", &parsed.to_string())?;
                        println!("  5-hour burst cap set to {}", format_tokens(parsed));
                    } else {
                        println!("  Invalid: {b}  (use e.g. 500M, 1B)");
                    }
                }
                if let Some(p) = plan {
                    db::set_config(&conn, "plan_name", &p)?;
                    println!("  Plan set to: {p}");
                }
            }
        }
        Commands::Calibrate { weekly_pct, burst_pct } => {
            if weekly_pct.is_none() && burst_pct.is_none() {
                // Show current calibration
                let cal_ts = db::get_config(&conn, "calibration_timestamp").unwrap_or_else(|| "never".to_string());
                let cal_wk_actual = db::get_config(&conn, "cal_actual_weekly_pct")
                    .and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let cal_5h_actual = db::get_config(&conn, "cal_actual_5h_pct")
                    .and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let cal_wk_tracked = db::get_config(&conn, "cal_tracked_weekly_tokens")
                    .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                let cal_5h_tracked = db::get_config(&conn, "cal_tracked_5h_tokens")
                    .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                println!("{}", "  costguard-pulse calibration".cyan().bold());
                println!("  {}", "─".repeat(52).dimmed());
                println!("  Last calibrated: {}", cal_ts);
                println!("  Weekly: actual {cal_wk_actual:.1}% (tracked {} at cal)", format_tokens(cal_wk_tracked));
                println!("  5-hour: actual {cal_5h_actual:.1}% (tracked {} at cal)", format_tokens(cal_5h_tracked));
                println!();
                println!("  To calibrate from claude.ai/settings/usage:");
                println!("    costguard-pulse calibrate --weekly-pct 33 --burst-pct 65");
            } else {
                let budget = db::get_plan_budget(&conn);
                let now = Utc::now();
                let week_start = (now - Duration::days(7)).format("%Y-%m-%dT%H:%M:%S").to_string();
                let five_hr_start = (now - Duration::hours(5)).format("%Y-%m-%dT%H:%M:%S").to_string();
                let week_tokens = db::tokens_in_window(&conn, &week_start);
                let five_hr_tokens = db::tokens_in_window(&conn, &five_hr_start);
                let tracked_week_pct = week_tokens as f64 / budget.weekly_tokens as f64 * 100.0;
                let tracked_5h_pct = five_hr_tokens as f64 / budget.five_hr_tokens as f64 * 100.0;

                if let Some(actual_wk) = weekly_pct {
                    db::set_config(&conn, "cal_actual_weekly_pct", &format!("{:.2}", actual_wk))?;
                    db::set_config(&conn, "cal_tracked_weekly_tokens", &week_tokens.to_string())?;
                    println!("  Weekly: tracked {tracked_week_pct:.1}% → actual {actual_wk:.1}% (gap {:+.1}%)", actual_wk - tracked_week_pct);
                }
                if let Some(actual_5h) = burst_pct {
                    db::set_config(&conn, "cal_actual_5h_pct", &format!("{:.2}", actual_5h))?;
                    db::set_config(&conn, "cal_tracked_5h_tokens", &five_hr_tokens.to_string())?;
                    println!("  5-hour: tracked {tracked_5h_pct:.1}% → actual {actual_5h:.1}% (gap {:+.1}%)", actual_5h - tracked_5h_pct);
                    println!("  → snapshot-delta: your tracked usage adds on top of {actual_5h:.0}%");
                    println!("  → valid for ~5h (rolling window), recalibrate when stale");
                }
                let ts = now.format("%Y-%m-%dT%H:%M:%S").to_string();
                db::set_config(&conn, "calibration_timestamp", &ts)?;
                println!("  Calibrated at {ts}");
            }
        }
        Commands::SyncRemote { host, user } => {
            println!("{}", "  costguard-pulse sync-remote".cyan().bold());
            println!("  {}", "─".repeat(52).dimmed());
            println!("  Host: {user}@{host}");

            // Find Claude Code transcript directories on remote host
            let remote_dir = format!("{user}@{host}");
            let cmd = format!(
                "ssh -o ConnectTimeout=10 -o BatchMode=yes {} 'find ~/.claude/projects -name \"*.jsonl\" -newer ~/.claude/projects/.last_sync 2>/dev/null | head -50 || find ~/.claude/projects -name \"*.jsonl\" -mtime -7 2>/dev/null | head -50'",
                remote_dir
            );
            let output = std::process::Command::new("bash")
                .args(["-c", &cmd])
                .output();

            match output {
                Ok(result) => {
                    let stdout = String::from_utf8_lossy(&result.stdout);
                    let files: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
                    println!("  Found {} transcript files on {host}", files.len());

                    if files.is_empty() {
                        println!("  No new transcripts to sync.");
                    } else {
                        // Sync each transcript to local temp dir and parse
                        let sync_dir = std::env::temp_dir().join("costguard-pulse-remote-sync");
                        std::fs::create_dir_all(&sync_dir).ok();

                        let mut synced = 0;
                        for remote_file in &files {
                            let local_file = sync_dir.join(
                                remote_file.replace('/', "_").trim_start_matches('_')
                            );
                            let scp_cmd = format!(
                                "scp -o ConnectTimeout=10 -o BatchMode=yes {}:{} {} 2>/dev/null",
                                remote_dir, remote_file, local_file.display()
                            );
                            if std::process::Command::new("bash")
                                .args(["-c", &scp_cmd])
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false)
                            {
                                synced += 1;
                            }
                        }
                        println!("  Synced {synced}/{} transcript files", files.len());
                        println!("  Run 'costguard-pulse sync' to parse them into the database.");
                    }
                }
                Err(e) => {
                    println!("  {}: {e}", "SSH failed".red());
                    println!("  Ensure SSH key auth works: ssh {remote_dir} 'echo ok'");
                }
            }
        }
        Commands::Doctor { errors } => cmd_doctor(&conn, errors)?,
        Commands::InstallTimer => cmd_install_timer()?,
        Commands::ProjectCosts { period } => cmd_project_costs(&conn, &period)?,
        Commands::TaskCosts { period } => cmd_task_costs(&conn, &period)?,
        Commands::Anomalies { period } => cmd_anomalies(&conn, &period)?,
    }

    Ok(())
}

// -----------------------------------------------------------------------
// Phase 3: Project costs, task costs, anomaly detection
// -----------------------------------------------------------------------

fn cmd_project_costs(conn: &Connection, period: &str) -> Result<()> {
    let since = period_to_timestamp(period);
    let costs = db::project_costs(conn, &since);

    if costs.is_empty() {
        println!("{}", "No session data for this period.".dimmed());
        return Ok(());
    }

    println!("{}", "Project Cost Allocation".bold().cyan());
    println!("{}", format!("Period: {period}").dimmed());
    println!();

    let mut table = comfy_table::Table::new();
    table.set_header(vec!["Project", "Sessions", "Cost ($)", "Output Tokens", "Commits", "$/Commit"]);
    table.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED);

    let mut total_cost = 0.0;
    for c in &costs {
        total_cost += c.total_cost;
        let project_short = c.project.split('/').last().unwrap_or(&c.project);
        table.add_row(vec![
            project_short.to_string(),
            c.sessions.to_string(),
            format!("{:.2}", c.total_cost),
            format_tokens(c.total_output_tokens),
            c.commits.to_string(),
            if c.cost_per_commit > 0.0 { format!("{:.3}", c.cost_per_commit) } else { "-".into() },
        ]);
    }

    println!("{table}");
    println!("Total: {}", format!("${:.2}", total_cost).bold().green());
    Ok(())
}

fn cmd_task_costs(conn: &Connection, period: &str) -> Result<()> {
    let since = period_to_timestamp(period);
    let costs = db::task_costs(conn, &since);

    if costs.is_empty() {
        println!("{}", "No task cost data for this period.".dimmed());
        println!("{}", "Hint: Set SWARM_TASK_ID env var in Claude Code sessions to track per-task costs".dimmed());
        return Ok(());
    }

    println!("{}", "Task Cost Breakdown".bold().cyan());
    println!();

    let mut table = comfy_table::Table::new();
    table.set_header(vec!["Task ID", "Project", "Sessions", "Cost ($)", "Output Tokens"]);
    table.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED);

    for c in &costs {
        let project_short = c.project.split('/').last().unwrap_or(&c.project);
        table.add_row(vec![
            c.task_id.clone(),
            project_short.to_string(),
            c.sessions.to_string(),
            format!("{:.2}", c.total_cost),
            format_tokens(c.total_output_tokens),
        ]);
    }

    println!("{table}");
    Ok(())
}

fn cmd_anomalies(conn: &Connection, period: &str) -> Result<()> {
    let since = period_to_timestamp(period);
    let anomalies = db::cost_anomalies(conn, &since);

    // Also check burst window
    let burst_pct = db::burst_window_pct(conn);

    println!("{}", "Cost Anomaly Detection".bold().cyan());
    println!();

    if burst_pct > 80.0 {
        println!("{}", format!(
            "  WARNING: 5-hour burst window at {:.0}% — approaching rate limit",
            burst_pct
        ).bold().red());
        println!();
    } else if burst_pct > 50.0 {
        println!("{}", format!(
            "  5-hour burst window: {:.0}%", burst_pct
        ).yellow());
        println!();
    }

    if anomalies.is_empty() {
        println!("{}", "  No cost anomalies detected.".green());
        return Ok(());
    }

    println!("{}", format!("  {} anomalous session(s) detected:", anomalies.len()).bold().yellow());
    println!();

    let mut table = comfy_table::Table::new();
    table.set_header(vec!["Session", "Project", "Cost ($)", "Mean ($)", "Z-Score", "Started"]);
    table.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED);

    for a in &anomalies {
        let project_short = a.project.split('/').last().unwrap_or(&a.project);
        let started_short = if a.started_at.len() > 16 { &a.started_at[..16] } else { &a.started_at };
        table.add_row(vec![
            a.session_id[..8.min(a.session_id.len())].to_string(),
            project_short.to_string(),
            format!("{:.2}", a.cost),
            format!("{:.2}", a.mean_cost),
            format!("{:.1}σ", a.z_score),
            started_short.to_string(),
        ]);
    }

    println!("{table}");
    Ok(())
}
