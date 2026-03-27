//! Model routing logic — model selection based on task complexity.
//!
//! Embeds routing rules so costguard-pulse can log WHY a particular model
//! was chosen for each subagent.

/// Recommend a model for a subagent task based on description keywords.
pub fn recommend_model(task_description: &str) -> (&'static str, &'static str) {
    let desc = task_description.to_lowercase();

    // Tier 1: Haiku — cheap research, search, simple lookups
    let haiku_keywords = [
        "search", "find", "list", "check", "verify", "look up",
        "web search", "grep", "glob", "explore", "count",
    ];
    for kw in &haiku_keywords {
        if desc.contains(kw) {
            return ("haiku", "keyword match: simple search/lookup task");
        }
    }

    // Tier 2: Sonnet — moderate reasoning, code generation, analysis
    let sonnet_keywords = [
        "generate", "write", "create", "build", "implement",
        "refactor", "review", "analyze", "fix", "debug",
        "test", "migrate",
    ];
    for kw in &sonnet_keywords {
        if desc.contains(kw) {
            return ("sonnet", "keyword match: code generation or analysis task");
        }
    }

    // Tier 3: Opus — complex reasoning, architecture, multi-step
    let opus_keywords = [
        "architect", "design", "plan", "complex", "multi-step",
        "security audit", "tax", "legal", "financial", "research",
        "evaluate", "compare",
    ];
    for kw in &opus_keywords {
        if desc.contains(kw) {
            return ("opus", "keyword match: complex reasoning required");
        }
    }

    // Default: sonnet (middle tier)
    ("sonnet", "default: no keyword match, using middle tier")
}

/// Estimate cost savings from routing to a cheaper model.
pub fn estimate_savings(
    recommended: &str,
    actual: &str,
    output_tokens: i64,
) -> f64 {
    let prices: std::collections::HashMap<&str, f64> = [
        ("haiku", 5.0),
        ("sonnet", 15.0),
        ("opus", 25.0),
    ].into();

    let rec_price = prices.get(recommended).copied().unwrap_or(15.0);
    let act_price = prices.get(actual).copied().unwrap_or(15.0);

    // Savings = (actual_price - recommended_price) * tokens / 1M
    (act_price - rec_price) * output_tokens as f64 / 1_000_000.0
}

/// Recommend model for the overall session based on task type.
pub fn recommend_session_model(project: &str, context: &str) -> (&'static str, &'static str) {
    let ctx = format!("{} {}", project, context).to_lowercase();

    // Simple file ops, git, mechanical work → Sonnet
    if ctx.contains("commit") || ctx.contains("push") || ctx.contains("sync")
        || ctx.contains("format") || ctx.contains("lint")
    {
        return ("sonnet", "mechanical task: git/format/lint");
    }

    // Research, web search → Haiku
    if ctx.contains("search") || ctx.contains("look up") || ctx.contains("check version") {
        return ("haiku", "simple research task");
    }

    // Default for development work → Opus
    ("opus", "development work: full reasoning needed")
}
