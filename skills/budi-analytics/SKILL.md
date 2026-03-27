---
name: budi-analytics
description: >
  budi token/cost analytics for Claude Code — dashboard access, CLI queries,
  cost analysis patterns, session tracking. Trigger on: "budi", "token cost",
  "how much did that cost", "session cost", "spending", "token analytics",
  "cost dashboard", "usage stats", or any question about Claude Code token
  consumption or spending patterns.
---

# budi Analytics

budi (WakaTime for Claude Code) tracks tokens, costs, and file activity per session.

## Quick Reference

```bash
# Dashboard (web UI)
open http://127.0.0.1:7878/dashboard

# CLI stats
budi stats                    # summary for current repo
budi stats --all              # all repos
budi stats --json             # machine-readable
budi sessions                 # list sessions
budi cost                     # cost breakdown
budi doctor                   # health check
```

## Architecture

- Daemon: `budi-daemon` on 127.0.0.1:7878 (started automatically)
- DB: `~/.local/share/budi/repos/<repo-hash>/budi.db` (SQLite, mode 600)
- Hooks: 5 events in `~/.claude/settings.json` (SessionStart, UserPromptSubmit, PostToolUse, SubagentStart, Stop)
- Status line: live cost shown in Claude Code terminal

## What It Tracks

| Data | Source Hook | Stored? |
|---|---|---|
| Session start/end | SessionStart, Stop | Yes |
| Prompt text | UserPromptSubmit | Yes (privacy note: prompts in SQLite) |
| Token counts | UserPromptSubmit | Yes |
| Model used | UserPromptSubmit | Yes |
| Cost estimate | Computed from model + tokens | Yes |
| Files touched | PostToolUse (Read/Write/Edit/Glob) | Yes (paths only, not content) |
| Subagent spawns | SubagentStart | Yes |

## Cost Model

budi uses Anthropic's published pricing:
- Haiku 4.5: $1/$5 per MTok (in/out)
- Sonnet 4.6: $3/$15 per MTok
- Opus 4.6: $5/$25 per MTok

## Weekly Cost Review Checklist

1. Run `budi stats --all` — compare session spend vs. prior week
2. Check `budi cost` — confirm Haiku handles >=50% of subagent calls
3. Verify no Opus usage for mechanical tasks (git, file reads, formatting)
4. Look for sessions with unusually high cost — investigate what drove it

## Troubleshooting

```bash
budi doctor                   # check hooks, daemon, config
systemctl --user status budi-daemon  # if daemon isn't running
budi-daemon &                 # start manually
```
