# CostGuard — Claude Code Cost Optimization Toolkit

## Status
- `status: PUBLIC-RELEASED`
- `last_verified: 2026-04-17`
- `owner: josh`
- `tier: infra`

## Project Location
/opt/claude-code-costguard

## Purpose
Open-source toolkit for reducing Claude Code session costs by 40-70% through
intelligent model routing, subagent delegation, and real-time analytics.

## Structure
- `skills/` — Claude Code skills (token-miser, session-miser, budi-analytics)
- `hooks/` — Shell hooks for settings.json (subagent routing, cost tracking)
- `analytics/costguard-pulse/` — Rust CLI for session analytics
- `config/` — Settings.json snippets for hook wiring
- `install.sh` — One-command installer

## Key Rules
- This is a PUBLIC repo — no PII, no internal IPs, no project-specific references
- All skills are sanitized copies — never reference Hydra, Christi, STR, or internal projects
- budi is a third-party dependency, not included — only documented
- costguard-pulse is a sanitized fork of hydra-pulse

## Build & Test
```bash
cd analytics/costguard-pulse && cargo test
cargo build --release
```
