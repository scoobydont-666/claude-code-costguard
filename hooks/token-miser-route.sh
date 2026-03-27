#!/usr/bin/env bash
set -euo pipefail

# Token-miser subagent router
# PreToolUse hook for Agent tool — sets model param based on task profile
# Returns JSON with hookSpecificOutput.updatedInput to inject model selection
#
# Install: Copy to ~/.claude/hooks/ and add to settings.json (see config/settings-snippet.json)

INPUT=$(cat)

# If model is already explicitly set, don't override
EXISTING_MODEL=$(echo "$INPUT" | jq -r '.tool_input.model // empty' | grep -v '^null$' || true)
if [[ -n "$EXISTING_MODEL" ]]; then
    exit 0
fi

SUBAGENT_TYPE=$(echo "$INPUT" | jq -r '.tool_input.subagent_type // "general-purpose"')
DESC=$(echo "$INPUT" | jq -r '.tool_input.description // ""' | tr '[:upper:]' '[:lower:]')

# Route based on subagent type + description keywords
pick_model() {
    # Explore agents are read-only search — always haiku
    if [[ "$SUBAGENT_TYPE" == "Explore" ]]; then
        echo "haiku"
        return
    fi

    # Plan agents need reasoning — opus
    if [[ "$SUBAGENT_TYPE" == "Plan" ]]; then
        echo "opus"
        return
    fi

    # Guide/help agents — haiku
    if [[ "$SUBAGENT_TYPE" == "claude-code-guide" || "$SUBAGENT_TYPE" == "statusline-setup" ]]; then
        echo "haiku"
        return
    fi

    # Keyword-based routing for general-purpose and custom agents
    local desc="$1"

    # Opus-tier first: judgment-heavy, open-ended, architectural
    local opus_patterns='\b(architect|design|plan impl|refactor|debug complex|security review|write complex)\b'
    if echo "$desc" | grep -qiP "$opus_patterns"; then
        echo "opus"
        return
    fi

    # Sonnet-tier: synthesis, generation, reasoning tasks
    local sonnet_patterns='\b(research|analyze|implement|generate|build|create|write|debug|review|compare|evaluate|migrate|convert|refactor)\b'
    if echo "$desc" | grep -qiP "$sonnet_patterns"; then
        echo "sonnet"
        return
    fi

    # Haiku-tier: everything else (mechanical, deterministic, read-only)
    echo "haiku"
}

MODEL=$(pick_model "$DESC")

# Validate model is in allowlist
if [[ ! "$MODEL" =~ ^(haiku|sonnet|opus)$ ]]; then
    MODEL="sonnet"
fi

# Build updatedInput with model injected
UPDATED=$(echo "$INPUT" | jq --arg model "$MODEL" '.tool_input + {model: $model}')

# Return hook output
jq -n --arg model "$MODEL" --argjson updated "$UPDATED" '{
    "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": $updated
    }
}'
