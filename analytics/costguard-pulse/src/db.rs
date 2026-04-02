use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    hostname TEXT NOT NULL,
    project TEXT,
    model TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    total_input_tokens INTEGER DEFAULT 0,
    total_output_tokens INTEGER DEFAULT 0,
    total_cache_read INTEGER DEFAULT 0,
    total_cache_write INTEGER DEFAULT 0,
    total_cost_usd REAL DEFAULT 0.0,
    git_branch TEXT,
    swarm_task_id TEXT,
    transcript_path TEXT
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    role TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_read INTEGER DEFAULT 0,
    cache_write INTEGER DEFAULT 0,
    cost_usd REAL DEFAULT 0.0,
    project TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS tool_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    duration_ms INTEGER
);

CREATE TABLE IF NOT EXISTS subagents (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    parent_message_id TEXT,
    model TEXT NOT NULL,
    recommended_model TEXT,
    routing_reason TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cost_usd REAL DEFAULT 0.0,
    task_description TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS skill_activations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    skill_name TEXT NOT NULL,
    activation_type TEXT NOT NULL,
    timestamp TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS commits (
    hash TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    project TEXT NOT NULL,
    message TEXT NOT NULL,
    files_changed INTEGER DEFAULT 0,
    timestamp TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_pricing (
    model TEXT PRIMARY KEY,
    input_per_mtok REAL NOT NULL,
    output_per_mtok REAL NOT NULL,
    cache_write_per_mtok REAL NOT NULL,
    cache_read_per_mtok REAL NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_errors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    error_message TEXT NOT NULL,
    timestamp TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_lock (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    pid INTEGER NOT NULL,
    started_at TEXT NOT NULL
);
"#;

const SEED_PRICING: &str = r#"
INSERT OR REPLACE INTO model_pricing VALUES
    ('claude-opus-4-6', 5.0, 25.0, 6.25, 0.50, '2026-03-23'),
    ('claude-sonnet-4-6', 3.0, 15.0, 3.75, 0.30, '2026-03-23'),
    ('claude-haiku-4-5', 1.0, 5.0, 1.25, 0.10, '2026-03-23'),
    ('claude-haiku-4-5-20251001', 1.0, 5.0, 1.25, 0.10, '2026-03-23');
"#;

pub fn db_path() -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("costguard-pulse");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("pulse.db")
}

pub fn open() -> Result<Connection> {
    let path = db_path();
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(SEED_PRICING)?;
    // Migrations: add columns if missing (existing DBs)
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN transcript_path TEXT;").ok();
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN last_synced_at TEXT;").ok();
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN prompt_count INTEGER DEFAULT 0;").ok();
    conn.execute_batch("ALTER TABLE tool_usage ADD COLUMN file_path TEXT;").ok();
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN subagent_input_tokens INTEGER DEFAULT 0;").ok();
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN subagent_output_tokens INTEGER DEFAULT 0;").ok();
    conn.execute_batch("ALTER TABLE sessions ADD COLUMN subagent_cost_usd REAL DEFAULT 0.0;").ok();
    Ok(conn)
}

pub fn get_config(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM config WHERE key = ?1",
        [key],
        |row| row.get(0),
    ).ok()
}

pub fn set_config(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Plan budget configuration.
pub struct PlanBudget {
    pub plan_name: String,
    pub weekly_tokens: i64,
    pub five_hr_tokens: i64,
}

/// Get plan budget config. Defaults based on Max plan estimates.
/// User calibrates via `costguard-pulse budget --weekly 4B --five-hr 500M`
pub fn get_plan_budget(conn: &Connection) -> PlanBudget {
    PlanBudget {
        plan_name: get_config(conn, "plan_name")
            .unwrap_or_else(|| "max".to_string()),
        weekly_tokens: get_config(conn, "weekly_budget_tokens")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(4_200_000_000), // ~4.2B estimated from real usage
        five_hr_tokens: get_config(conn, "five_hr_budget_tokens")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(500_000_000), // ~500M estimated
    }
}

/// Query total tokens consumed in a rolling window.
pub fn tokens_in_window(conn: &Connection, window_start: &str) -> i64 {
    conn.query_row(
        "SELECT COALESCE(SUM(total_input_tokens + total_cache_read + total_cache_write + total_output_tokens), 0) FROM sessions WHERE started_at >= ?1",
        [window_start],
        |row| row.get(0),
    ).unwrap_or(0)
}

/// Query tokens per model in a rolling window.
pub struct ModelTokens {
    pub model: String,
    pub tokens: i64,
}

pub fn tokens_in_window_by_model(conn: &Connection, window_start: &str) -> Vec<ModelTokens> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(model, 'unknown'), \
         COALESCE(SUM(total_input_tokens + total_cache_read + total_cache_write + total_output_tokens), 0) \
         FROM sessions WHERE started_at >= ?1 GROUP BY model ORDER BY 2 DESC"
    ).unwrap();
    stmt.query_map([window_start], |row| {
        Ok(ModelTokens { model: row.get(0)?, tokens: row.get(1)? })
    }).unwrap().filter_map(|r| r.ok()).collect()
}

/// Query the oldest session start time in a rolling window.
pub fn oldest_session_in_window(conn: &Connection, window_start: &str) -> Option<String> {
    conn.query_row(
        "SELECT started_at FROM sessions WHERE started_at >= ?1 ORDER BY started_at ASC LIMIT 1",
        [window_start],
        |row| row.get(0),
    ).ok()
}

/// Query cache hit rate in a rolling window.
pub fn cache_hit_in_window(conn: &Connection, window_start: &str) -> f64 {
    conn.query_row(
        "SELECT CASE WHEN SUM(total_cache_read)+SUM(total_cache_write)+SUM(total_input_tokens) > 0 THEN CAST(SUM(total_cache_read) AS REAL)/(SUM(total_cache_read)+SUM(total_cache_write)+SUM(total_input_tokens))*100 ELSE 0 END FROM sessions WHERE started_at >= ?1",
        [window_start],
        |row| row.get(0),
    ).unwrap_or(0.0)
}

/// Try to acquire the sync lock. Returns true if acquired.
/// Checks if existing lock holder PID is still alive; steals stale locks.
pub fn try_acquire_sync_lock(conn: &Connection) -> bool {
    let pid = std::process::id() as i64;
    let now = chrono::Utc::now().to_rfc3339();

    // Check for existing lock
    let existing: Option<i64> = conn.query_row(
        "SELECT pid FROM sync_lock WHERE id = 1",
        [],
        |row| row.get(0),
    ).ok();

    if let Some(existing_pid) = existing {
        // Check if the process is still alive
        let alive = std::path::Path::new(&format!("/proc/{existing_pid}")).exists();
        if alive && existing_pid != pid {
            return false; // Another live process holds the lock
        }
        // Stale lock or same process — overwrite
    }

    conn.execute(
        "INSERT OR REPLACE INTO sync_lock (id, pid, started_at) VALUES (1, ?1, ?2)",
        rusqlite::params![pid, now],
    ).is_ok()
}

/// Release the sync lock (only if we hold it).
pub fn release_sync_lock(conn: &Connection) {
    let pid = std::process::id() as i64;
    conn.execute(
        "DELETE FROM sync_lock WHERE id = 1 AND pid = ?1",
        [pid],
    ).ok();
}

/// Log a sync error for a session.
pub fn log_sync_error(conn: &Connection, session_id: &str, error: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sync_errors (session_id, error_message, timestamp) VALUES (?1, ?2, ?3)",
        rusqlite::params![session_id, error, now],
    ).ok();
}

// -----------------------------------------------------------------------
// Phase 3: Project-aware cost allocation + anomaly detection
// -----------------------------------------------------------------------

/// Per-project cost summary.
pub struct ProjectCost {
    pub project: String,
    pub sessions: i64,
    pub total_cost: f64,
    pub total_output_tokens: i64,
    pub commits: i64,
    pub cost_per_commit: f64,
}

/// Query cost breakdown by project.
pub fn project_costs(conn: &Connection, since: &str) -> Vec<ProjectCost> {
    let mut stmt = conn.prepare(
        "SELECT
            COALESCE(s.project, '(unknown)') as project,
            COUNT(DISTINCT s.id) as sessions,
            COALESCE(SUM(s.total_cost_usd), 0.0) as total_cost,
            COALESCE(SUM(s.total_output_tokens), 0) as output_tokens,
            (SELECT COUNT(*) FROM commits c WHERE c.project = s.project AND c.timestamp >= ?1) as commit_count
         FROM sessions s
         WHERE s.started_at >= ?1
         GROUP BY s.project
         ORDER BY total_cost DESC"
    ).unwrap();

    let rows = stmt.query_map([since], |row| {
        let total_cost: f64 = row.get(2)?;
        let commits: i64 = row.get(4)?;
        Ok(ProjectCost {
            project: row.get(0)?,
            sessions: row.get(1)?,
            total_cost,
            total_output_tokens: row.get(3)?,
            commits,
            cost_per_commit: if commits > 0 { total_cost / commits as f64 } else { 0.0 },
        })
    }).unwrap();

    rows.filter_map(|r| r.ok()).collect()
}

/// Per-task cost summary (task ID correlation).
pub struct TaskCost {
    pub task_id: String,
    pub project: String,
    pub sessions: i64,
    pub total_cost: f64,
    pub total_output_tokens: i64,
}

/// Query cost breakdown by task ID (populated from SWARM_TASK_ID env var if present).
pub fn task_costs(conn: &Connection, since: &str) -> Vec<TaskCost> {
    let mut stmt = conn.prepare(
        "SELECT
            COALESCE(swarm_task_id, '(none)') as task_id,
            COALESCE(project, '(unknown)') as project,
            COUNT(*) as sessions,
            COALESCE(SUM(total_cost_usd), 0.0) as total_cost,
            COALESCE(SUM(total_output_tokens), 0) as output_tokens
         FROM sessions
         WHERE started_at >= ?1 AND swarm_task_id IS NOT NULL AND swarm_task_id != ''
         GROUP BY swarm_task_id
         ORDER BY total_cost DESC"
    ).unwrap();

    let rows = stmt.query_map([since], |row| {
        Ok(TaskCost {
            task_id: row.get(0)?,
            project: row.get(1)?,
            sessions: row.get(2)?,
            total_cost: row.get(3)?,
            total_output_tokens: row.get(4)?,
        })
    }).unwrap();

    rows.filter_map(|r| r.ok()).collect()
}

/// Cost anomaly: a session that's >2 standard deviations from the mean.
pub struct CostAnomaly {
    pub session_id: String,
    pub project: String,
    pub cost: f64,
    pub mean_cost: f64,
    pub std_dev: f64,
    pub z_score: f64,
    pub started_at: String,
}

/// Detect cost anomalies (sessions >2 std devs from mean).
pub fn cost_anomalies(conn: &Connection, since: &str) -> Vec<CostAnomaly> {
    // First compute mean and stddev
    let stats: (f64, f64) = conn.query_row(
        "SELECT
            COALESCE(AVG(total_cost_usd), 0.0),
            COALESCE(
                SQRT(AVG(total_cost_usd * total_cost_usd) - AVG(total_cost_usd) * AVG(total_cost_usd)),
                0.0
            )
         FROM sessions WHERE started_at >= ?1 AND total_cost_usd > 0",
        [since],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((0.0, 0.0));

    let (mean, std_dev) = stats;
    if std_dev < 0.001 {
        return vec![]; // Not enough variance
    }

    let threshold = mean + 2.0 * std_dev;

    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(project, ''), total_cost_usd, started_at
         FROM sessions
         WHERE started_at >= ?1 AND total_cost_usd > ?2
         ORDER BY total_cost_usd DESC"
    ).unwrap();

    let rows = stmt.query_map(rusqlite::params![since, threshold], |row| {
        let cost: f64 = row.get(2)?;
        Ok(CostAnomaly {
            session_id: row.get(0)?,
            project: row.get(1)?,
            cost,
            mean_cost: mean,
            std_dev,
            z_score: (cost - mean) / std_dev,
            started_at: row.get(3)?,
        })
    }).unwrap();

    rows.filter_map(|r| r.ok()).collect()
}

// -----------------------------------------------------------------------
// Fleet cost aggregation
// -----------------------------------------------------------------------

/// Per-host cost summary.
pub struct HostCost {
    pub hostname: String,
    pub sessions: i64,
    pub total_cost: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub cache_hit_pct: f64,
    pub projects: i64,
}

/// Aggregate costs by hostname (useful for multi-machine setups).
pub fn fleet_costs(conn: &Connection, since: &str) -> Vec<HostCost> {
    let mut stmt = conn.prepare(
        "SELECT
            COALESCE(hostname, '(local)') as host,
            COUNT(*) as sessions,
            COALESCE(SUM(total_cost_usd), 0.0) as total_cost,
            COALESCE(SUM(total_input_tokens), 0) as input_tokens,
            COALESCE(SUM(total_output_tokens), 0) as output_tokens,
            COALESCE(SUM(total_cache_read), 0) as cache_read,
            COALESCE(SUM(total_cache_write), 0) as cache_write,
            COUNT(DISTINCT project) as projects
         FROM sessions
         WHERE started_at >= ?1
         GROUP BY hostname
         ORDER BY total_cost DESC"
    ).unwrap();

    let rows = stmt.query_map([since], |row| {
        let cache_read: i64 = row.get(5)?;
        let cache_write: i64 = row.get(6)?;
        let input_tokens: i64 = row.get(3)?;
        let total = cache_read + cache_write + input_tokens;
        Ok(HostCost {
            hostname: row.get(0)?,
            sessions: row.get(1)?,
            total_cost: row.get(2)?,
            total_input_tokens: input_tokens,
            total_output_tokens: row.get(4)?,
            cache_hit_pct: if total > 0 { cache_read as f64 / total as f64 * 100.0 } else { 0.0 },
            projects: row.get(7)?,
        })
    }).unwrap();

    rows.filter_map(|r| r.ok()).collect()
}

// -----------------------------------------------------------------------
// Efficiency scoring
// -----------------------------------------------------------------------

/// Efficiency metrics for a project.
pub struct EfficiencyScore {
    pub project: String,
    pub tokens_per_commit: f64,
    pub cost_per_commit: f64,
    pub waste_ratio: f64,
    pub cache_hit_rate: f64,
    pub avg_session_cost: f64,
}

/// Calculate efficiency scores per project.
pub fn efficiency_scores(conn: &Connection, since: &str) -> Vec<EfficiencyScore> {
    let mut stmt = conn.prepare(
        "SELECT
            COALESCE(s.project, '(unknown)') as project,
            COUNT(DISTINCT s.id) as sessions,
            COALESCE(SUM(s.total_input_tokens + s.total_output_tokens + s.total_cache_read + s.total_cache_write), 0) as total_tokens,
            COALESCE(SUM(s.total_output_tokens), 0) as output_tokens,
            COALESCE(SUM(s.total_cache_read), 0) as cache_read,
            COALESCE(SUM(s.total_input_tokens + s.total_cache_read + s.total_cache_write), 0) as input_total,
            COALESCE(SUM(s.total_cost_usd), 0.0) as total_cost,
            (SELECT COUNT(*) FROM commits c WHERE c.project = s.project AND c.timestamp >= ?1) as commit_count,
            (SELECT COUNT(DISTINCT tu.session_id) FROM tool_usage tu JOIN sessions ss ON tu.session_id = ss.id WHERE ss.project = s.project AND ss.started_at >= ?1) as sessions_with_tools
         FROM sessions s
         WHERE s.started_at >= ?1
         GROUP BY s.project
         HAVING total_tokens > 0
         ORDER BY total_cost DESC"
    ).unwrap();

    let rows = stmt.query_map([since], |row| {
        let sessions: i64 = row.get(1)?;
        let total_tokens: i64 = row.get(2)?;
        let cache_read: i64 = row.get(4)?;
        let input_total: i64 = row.get(5)?;
        let total_cost: f64 = row.get(6)?;
        let commits: i64 = row.get(7)?;
        let sessions_with_tools: i64 = row.get(8)?;

        Ok(EfficiencyScore {
            project: row.get(0)?,
            tokens_per_commit: if commits > 0 { total_tokens as f64 / commits as f64 } else { 0.0 },
            cost_per_commit: if commits > 0 { total_cost / commits as f64 } else { 0.0 },
            waste_ratio: if sessions > 0 { (sessions - sessions_with_tools) as f64 / sessions as f64 } else { 0.0 },
            cache_hit_rate: if input_total > 0 { cache_read as f64 / input_total as f64 * 100.0 } else { 0.0 },
            avg_session_cost: if sessions > 0 { total_cost / sessions as f64 } else { 0.0 },
        })
    }).unwrap();

    rows.filter_map(|r| r.ok()).collect()
}

// -----------------------------------------------------------------------
// Budget alerts
// -----------------------------------------------------------------------

/// Budget alert for a project exceeding daily spend.
pub struct BudgetAlert {
    pub project: String,
    pub daily_limit: f64,
    pub daily_spent: f64,
    pub exceeded: bool,
}

/// Check per-project daily spend against configurable thresholds.
pub fn check_budget_alerts(conn: &Connection) -> Vec<BudgetAlert> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let default_limit: f64 = get_config(conn, "default_daily_budget")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50.0);

    let mut stmt = conn.prepare(
        "SELECT COALESCE(project, '(unknown)'), COALESCE(SUM(total_cost_usd), 0.0)
         FROM sessions WHERE started_at >= ?1 GROUP BY project ORDER BY 2 DESC"
    ).unwrap();

    let rows = stmt.query_map([&today], |row| {
        let project: String = row.get(0)?;
        let daily_spent: f64 = row.get(1)?;
        let key = format!("budget_{}", project.replace('/', "_"));
        let limit = get_config(conn, &key).and_then(|v| v.parse().ok()).unwrap_or(default_limit);
        Ok(BudgetAlert { project, daily_limit: limit, daily_spent, exceeded: daily_spent > limit })
    }).unwrap();

    rows.filter_map(|r| r.ok()).filter(|a| a.exceeded).collect()
}

/// Budget warning: percentage of 5-hour window consumed.
pub fn burst_window_pct(conn: &Connection) -> f64 {
    let budget = get_plan_budget(conn);
    if budget.five_hr_tokens == 0 { return 0.0; }

    let five_hrs_ago = (chrono::Utc::now() - chrono::Duration::hours(5)).to_rfc3339();
    let consumed = tokens_in_window(conn, &five_hrs_ago);
    (consumed as f64 / budget.five_hr_tokens as f64) * 100.0
}

pub fn compute_cost(
    conn: &Connection,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read: i64,
    cache_write: i64,
) -> f64 {
    if let Some((inp, out, cw, cr)) = lookup_pricing(conn, model) {
        return (input_tokens as f64 * inp
            + output_tokens as f64 * out
            + cache_write as f64 * cw
            + cache_read as f64 * cr)
            / 1_000_000.0;
    }
    0.0
}

/// Look up model pricing with fallback chain:
/// 1. Exact match
/// 2. Strip bracket annotation: "claude-opus-4-6[1m]" -> "claude-opus-4-6"
/// 3. Strip date suffix: "claude-opus-4-6-20260401" -> "claude-opus-4-6"
/// 4. Prefix match: find pricing row where model name is a prefix of the query
fn lookup_pricing(conn: &Connection, model: &str) -> Option<(f64, f64, f64, f64)> {
    // 1. Exact match
    if let Some(p) = query_pricing(conn, model) {
        return Some(p);
    }

    // 2. Strip bracket annotation: "claude-opus-4-6[1m]" -> "claude-opus-4-6"
    if let Some(pos) = model.find('[') {
        let stripped = &model[..pos];
        if stripped != model {
            if let Some(p) = query_pricing(conn, stripped) {
                return Some(p);
            }
        }
    }

    // 3. Strip date suffix: "claude-opus-4-6-20260401" -> "claude-opus-4-6"
    if let Some(pos) = model.rfind('-') {
        let suffix = &model[pos + 1..];
        if suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_digit()) {
            let stripped = &model[..pos];
            if let Some(p) = query_pricing(conn, stripped) {
                return Some(p);
            }
        }
    }

    // 4. Prefix match: find best pricing row where the pricing model is a prefix of the query
    let mut stmt = conn
        .prepare_cached(
            "SELECT input_per_mtok, output_per_mtok, cache_write_per_mtok, cache_read_per_mtok FROM model_pricing WHERE ?1 LIKE model || '%' ORDER BY LENGTH(model) DESC LIMIT 1",
        )
        .unwrap();

    stmt.query_row([model], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    }).ok()
}

fn query_pricing(conn: &Connection, model: &str) -> Option<(f64, f64, f64, f64)> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT input_per_mtok, output_per_mtok, cache_write_per_mtok, cache_read_per_mtok FROM model_pricing WHERE model = ?1",
        )
        .unwrap();

    stmt.query_row([model], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    }).ok()
}
