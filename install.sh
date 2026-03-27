#!/usr/bin/env bash
set -euo pipefail

# CostGuard installer — drops skills, hooks, and analytics into your Claude Code setup
# Usage: ./install.sh [--skills-only | --hooks-only | --analytics-only | --all]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILLS_DIR="$HOME/.claude/skills"
HOOKS_DIR="$HOME/.claude/hooks"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[+]${NC} $1"; }
warn()  { echo -e "${YELLOW}[!]${NC} $1"; }
error() { echo -e "${RED}[x]${NC} $1"; }

MODE="${1:---all}"

install_skills() {
    info "Installing skills to $SKILLS_DIR..."
    mkdir -p "$SKILLS_DIR"

    for skill_dir in "$SCRIPT_DIR"/skills/*/; do
        skill_name=$(basename "$skill_dir")
        target="$SKILLS_DIR/$skill_name"

        if [[ -d "$target" ]]; then
            warn "Skill '$skill_name' already exists — backing up to ${target}.bak"
            cp -r "$target" "${target}.bak"
        fi

        cp -r "$skill_dir" "$target"
        info "  Installed: $skill_name"
    done
}

install_hooks() {
    info "Installing hooks to $HOOKS_DIR..."
    mkdir -p "$HOOKS_DIR"

    for hook in "$SCRIPT_DIR"/hooks/*.sh; do
        hook_name=$(basename "$hook")
        target="$HOOKS_DIR/$hook_name"

        if [[ -f "$target" ]]; then
            warn "Hook '$hook_name' already exists — backing up to ${target}.bak"
            cp "$target" "${target}.bak"
        fi

        cp "$hook" "$target"
        chmod +x "$target"
        info "  Installed: $hook_name"
    done

    echo ""
    warn "IMPORTANT: You must manually add the hooks to your settings.json."
    warn "See config/settings-snippet.json for the required hook configuration."
    warn "Merge it into: ~/.claude/settings.json"
}

install_analytics() {
    info "Building costguard-pulse analytics..."

    if ! command -v cargo &>/dev/null; then
        error "Rust/Cargo not found. Install from https://rustup.rs/ first."
        return 1
    fi

    cd "$SCRIPT_DIR/analytics/costguard-pulse"
    cargo build --release 2>&1 | tail -3

    local bin_dir="$HOME/.local/bin"
    mkdir -p "$bin_dir"

    cp target/release/costguard-pulse "$bin_dir/"
    cp target/release/costguard-pulse-hook "$bin_dir/"
    info "  Installed: costguard-pulse → $bin_dir/"
    info "  Installed: costguard-pulse-hook → $bin_dir/"
    info "  Run 'costguard-pulse doctor' to verify."
}

check_deps() {
    info "Checking dependencies..."

    # jq is required for hooks
    if ! command -v jq &>/dev/null; then
        error "jq is required but not installed."
        error "  Ubuntu/Debian: sudo apt install jq"
        error "  macOS: brew install jq"
        exit 1
    fi
    info "  jq: found"

    # bc is used by cost tracker
    if ! command -v bc &>/dev/null; then
        warn "  bc: not found (subagent-cost-tracker.sh needs it)"
    else
        info "  bc: found"
    fi

    # budi is recommended but optional
    if command -v budi &>/dev/null; then
        info "  budi: found"
    else
        warn "  budi: not found (optional — install from https://github.com/ryanhoangt/budi)"
    fi
}

echo ""
echo "  CostGuard — Claude Code Cost Optimization Toolkit"
echo "  ================================================="
echo ""

check_deps

case "$MODE" in
    --skills-only)
        install_skills
        ;;
    --hooks-only)
        install_hooks
        ;;
    --analytics-only)
        install_analytics
        ;;
    --all)
        install_skills
        echo ""
        install_hooks
        echo ""
        install_analytics
        ;;
    *)
        echo "Usage: $0 [--skills-only | --hooks-only | --analytics-only | --all]"
        exit 1
        ;;
esac

echo ""
info "Installation complete."
info "See README.md for configuration and usage instructions."
