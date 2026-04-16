#!/usr/bin/env bash
set -euo pipefail

# SubagentStop hook — tracks subagent cost and usage
# Logs to ~/.claude/hooks/logs/subagent-costs.jsonl
# Pricing per MTok (verified 2026-03-17)
#
# Install: Copy to ~/.claude/hooks/ and add to settings.json (see config/settings-snippet.json)

LOG_DIR="$HOME/.claude/hooks/logs"
LOG_FILE="$LOG_DIR/subagent-costs.jsonl"
mkdir -p "$LOG_DIR"

INPUT=$(cat)

AGENT_ID=$(echo "$INPUT" | jq -r '.agent_id // "unknown"')
AGENT_TYPE=$(echo "$INPUT" | jq -r '.agent_type // "unknown"')
TRANSCRIPT_PATH=$(echo "$INPUT" | jq -r '.agent_transcript_path // empty')

# Try to extract token usage from transcript
INPUT_TOKENS=0
OUTPUT_TOKENS=0
MODEL="unknown"

if [[ -n "$TRANSCRIPT_PATH" && -f "$TRANSCRIPT_PATH" ]]; then
    # Parse JSONL transcript for usage data — sum all usage entries
    # Usage is nested at .message.usage (not top-level .usage)
    USAGE=$(jq -s '
        [.[] | select(.message.usage) | .message.usage] |
        {
            input_tokens: (map(.input_tokens // 0) | add // 0),
            output_tokens: (map(.output_tokens // 0) | add // 0),
            cache_read_input_tokens: (map(.cache_read_input_tokens // 0) | add // 0),
            cache_creation_input_tokens: (map(.cache_creation_input_tokens // 0) | add // 0)
        }
    ' "$TRANSCRIPT_PATH" 2>/dev/null || echo '{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}')

    INPUT_TOKENS=$(echo "$USAGE" | jq -r '.input_tokens // 0')
    OUTPUT_TOKENS=$(echo "$USAGE" | jq -r '.output_tokens // 0')
    # Ensure numeric
    [[ "$INPUT_TOKENS" =~ ^[0-9]+$ ]] || INPUT_TOKENS=0
    [[ "$OUTPUT_TOKENS" =~ ^[0-9]+$ ]] || OUTPUT_TOKENS=0

    # Extract model from first assistant message (nested at .message.model)
    MODEL=$(jq -r 'select(.message.model) | .message.model' "$TRANSCRIPT_PATH" 2>/dev/null | head -1 || echo "unknown")
    if [[ -z "$MODEL" || "$MODEL" == "null" ]]; then
        MODEL="unknown"
    fi
fi

# Calculate cost based on model
COST="0.0"
if [[ "$INPUT_TOKENS" -gt 0 || "$OUTPUT_TOKENS" -gt 0 ]]; then
    case "$MODEL" in
        *haiku*)
            COST=$(echo "scale=6; ($INPUT_TOKENS * 1.0 / 1000000) + ($OUTPUT_TOKENS * 5.0 / 1000000)" | bc 2>/dev/null) || COST="0.0"
            ;;
        *sonnet*)
            COST=$(echo "scale=6; ($INPUT_TOKENS * 3.0 / 1000000) + ($OUTPUT_TOKENS * 15.0 / 1000000)" | bc 2>/dev/null) || COST="0.0"
            ;;
        *opus*)
            COST=$(echo "scale=6; ($INPUT_TOKENS * 5.0 / 1000000) + ($OUTPUT_TOKENS * 25.0 / 1000000)" | bc 2>/dev/null) || COST="0.0"
            ;;
        *)
            COST=$(echo "scale=6; ($INPUT_TOKENS * 3.0 / 1000000) + ($OUTPUT_TOKENS * 15.0 / 1000000)" | bc 2>/dev/null) || COST="0.0"
            ;;
    esac
fi

# Log entry
jq -nc \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg agent_id "$AGENT_ID" \
    --arg agent_type "$AGENT_TYPE" \
    --arg model "$MODEL" \
    --argjson input_tokens "$INPUT_TOKENS" \
    --argjson output_tokens "$OUTPUT_TOKENS" \
    --arg cost_usd "$COST" \
    '{
        timestamp: $ts,
        agent_id: $agent_id,
        agent_type: $agent_type,
        model: $model,
        input_tokens: $input_tokens,
        output_tokens: $output_tokens,
        cost_usd: $cost_usd
    }' >> "$LOG_FILE"

# Output context back to main conversation (silent — no user-visible message)
jq -nc \
    --arg model "$MODEL" \
    --argjson input_tokens "$INPUT_TOKENS" \
    --argjson output_tokens "$OUTPUT_TOKENS" \
    --arg cost "$COST" \
    '{
        "hookSpecificOutput": {
            "hookEventName": "SubagentStop",
            "additionalContext": ("Subagent used " + $model + ": " + ($input_tokens|tostring) + " in / " + ($output_tokens|tostring) + " out = $" + $cost)
        }
    }'
