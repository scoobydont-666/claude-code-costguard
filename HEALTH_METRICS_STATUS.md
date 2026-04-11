# Health & Metrics Status — CostGuard

## Overview

CostGuard is a **CLI-only application** (not a web service). Health endpoints and Prometheus metrics are not applicable.

## Components

### costguard-pulse (Rust CLI)
- **Type**: Standalone CLI tool
- **Purpose**: Session analytics, cost tracking, model routing analysis
- **Status**: N/A for health/metrics endpoints (no HTTP server)

### token-miser (Skill)
- **Type**: Claude Code skill (Python)
- **Purpose**: Subagent model routing
- **Status**: N/A for health/metrics endpoints (no HTTP server)

### session-miser (Skill)
- **Type**: Claude Code skill (Python)
- **Purpose**: Session-level model optimization
- **Status**: N/A for health/metrics endpoints (no HTTP server)

## Usage

CostGuard is invoked via:

1. **Direct CLI invocation**:
   ```bash
   costguard-pulse stats --period today
   costguard-pulse cost --by session
   ```

2. **Claude Code hooks** (auto-invoked):
   - `token-miser-route.sh` — Subagent routing decision
   - `subagent-cost-tracker.sh` — Cost logging

## Observability

Metrics are captured via:
- **Hook logs**: CSV/JSONL in ~/.claude/hooks/logs/
- **costguard-pulse commands**: Live analysis from hook database
- **Session transcripts**: Raw token/cost data

For fleet-wide monitoring, aggregate costguard-pulse output via cron to a metrics service (Prometheus, ClickHouse, etc.).

## Public Repo Status

CostGuard is PUBLIC (MIT license). No security concerns with health/metrics documentation.
