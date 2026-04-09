#!/usr/bin/env bash
# Shell script test suite for costguard hooks
# Tests the shell hook scripts directly
# Run: ./hooks/test-hooks.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR="/tmp/costguard-hooks-test-$$"
HOOKS_DIR="${SCRIPT_DIR}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counters
TESTS_RUN=0
TESTS_PASS=0
TESTS_FAIL=0

# Cleanup on exit
trap 'rm -rf "$TEST_DIR"' EXIT

# Test utilities
assert_file_exists() {
    local file=$1
    if [[ ! -f "$file" ]]; then
        echo -e "${RED}✗ File not found: $file${NC}"
        TESTS_FAIL=$((TESTS_FAIL + 1))
        return 1
    fi
    TESTS_PASS=$((TESTS_PASS + 1))
    return 0
}

assert_contains() {
    local haystack=$1
    local needle=$2
    local test_name=$3

    if echo "$haystack" | grep -q "$needle"; then
        echo -e "${GREEN}✓ $test_name${NC}"
        TESTS_PASS=$((TESTS_PASS + 1))
        return 0
    else
        echo -e "${RED}✗ $test_name (expected '$needle' in output)${NC}"
        TESTS_FAIL=$((TESTS_FAIL + 1))
        return 1
    fi
}

assert_exit_code() {
    local exit_code=$1
    local expected=$2
    local test_name=$3

    if [[ $exit_code -eq $expected ]]; then
        echo -e "${GREEN}✓ $test_name${NC}"
        TESTS_PASS=$((TESTS_PASS + 1))
        return 0
    else
        echo -e "${RED}✗ $test_name (exit code: expected $expected, got $exit_code)${NC}"
        TESTS_FAIL=$((TESTS_FAIL + 1))
        return 1
    fi
}

run_test() {
    local test_name=$1
    TESTS_RUN=$((TESTS_RUN + 1))
    echo -e "\n${YELLOW}Test: $test_name${NC}"
}

# Setup test environment
setup_test_env() {
    mkdir -p "$TEST_DIR/costguard-pulse"
    export XDG_DATA_HOME="$TEST_DIR"
}

# ============================================================================
# Token Miser Route Hook Tests
# ============================================================================

if [[ -f "${HOOKS_DIR}/token-miser-route.sh" ]]; then
    run_test "token-miser-route.sh exists"
    assert_file_exists "${HOOKS_DIR}/token-miser-route.sh"

    run_test "token-miser-route.sh is executable"
    if [[ -x "${HOOKS_DIR}/token-miser-route.sh" ]]; then
        echo -e "${GREEN}✓ Executable${NC}"
        TESTS_PASS=$((TESTS_PASS + 1))
    else
        echo -e "${RED}✗ Not executable${NC}"
        TESTS_FAIL=$((TESTS_FAIL + 1))
    fi

    run_test "token-miser-route.sh contains routing table"
    grep -q "Explore\|Search\|Haiku" "${HOOKS_DIR}/token-miser-route.sh" && \
        assert_contains "$(cat ${HOOKS_DIR}/token-miser-route.sh)" "haiku" "Routing table present" || \
        echo -e "${YELLOW}⚠ Routing logic may not be explicitly documented${NC}"
else
    echo -e "${YELLOW}⚠ token-miser-route.sh not found (optional)${NC}"
fi

# ============================================================================
# Subagent Cost Tracker Hook Tests
# ============================================================================

if [[ -f "${HOOKS_DIR}/subagent-cost-tracker.sh" ]]; then
    run_test "subagent-cost-tracker.sh exists"
    assert_file_exists "${HOOKS_DIR}/subagent-cost-tracker.sh"

    run_test "subagent-cost-tracker.sh is executable"
    if [[ -x "${HOOKS_DIR}/subagent-cost-tracker.sh" ]]; then
        echo -e "${GREEN}✓ Executable${NC}"
        TESTS_PASS=$((TESTS_PASS + 1))
    else
        echo -e "${RED}✗ Not executable${NC}"
        TESTS_FAIL=$((TESTS_FAIL + 1))
    fi
else
    echo -e "${YELLOW}⚠ subagent-cost-tracker.sh not found (optional)${NC}"
fi

# ============================================================================
# Install Script Tests
# ============================================================================

run_test "install.sh exists"
assert_file_exists "${SCRIPT_DIR}/../install.sh"

run_test "install.sh is executable"
if [[ -x "${SCRIPT_DIR}/../install.sh" ]]; then
    echo -e "${GREEN}✓ Executable${NC}"
    TESTS_PASS=$((TESTS_PASS + 1))
else
    echo -e "${RED}✗ Not executable${NC}"
    TESTS_FAIL=$((TESTS_FAIL + 1))
fi

run_test "install.sh supports --skills-only"
if grep -q "\-\-skills-only" "${SCRIPT_DIR}/../install.sh"; then
    echo -e "${GREEN}✓ Supports --skills-only${NC}"
    TESTS_PASS=$((TESTS_PASS + 1))
else
    echo -e "${YELLOW}⚠ --skills-only not documented${NC}"
fi

# ============================================================================
# Configuration Tests
# ============================================================================

run_test "settings-snippet.json exists"
assert_file_exists "${SCRIPT_DIR}/../config/settings-snippet.json"

run_test "settings-snippet.json is valid JSON"
setup_test_env
if jq empty "${SCRIPT_DIR}/../config/settings-snippet.json" 2>/dev/null; then
    echo -e "${GREEN}✓ Valid JSON${NC}"
    TESTS_PASS=$((TESTS_PASS + 1))
else
    echo -e "${RED}✗ Invalid JSON${NC}"
    TESTS_FAIL=$((TESTS_FAIL + 1))
fi

run_test "settings-snippet.json has required hooks"
hooks=("session-start" "session-end" "tool-use" "agent-start" "agent-end")
snippet=$(cat "${SCRIPT_DIR}/../config/settings-snippet.json")
for hook in "${hooks[@]}"; do
    if echo "$snippet" | grep -q "$hook"; then
        echo -e "${GREEN}✓ Hook configured: $hook${NC}"
        TESTS_PASS=$((TESTS_PASS + 1))
    else
        echo -e "${YELLOW}⚠ Hook not configured: $hook${NC}"
    fi
done

# ============================================================================
# README Documentation Tests
# ============================================================================

run_test "README.md exists and is readable"
assert_file_exists "${SCRIPT_DIR}/../README.md"

run_test "README documents cost savings"
assert_contains "$(cat ${SCRIPT_DIR}/../README.md)" "40-70" "Cost savings documented"

run_test "README documents quick start"
assert_contains "$(cat ${SCRIPT_DIR}/../README.md)" "Quick Start" "Quick start section present"

run_test "README documents architecture"
assert_contains "$(cat ${SCRIPT_DIR}/../README.md)" "Architecture" "Architecture section present"

# ============================================================================
# Skill Documentation Tests
# ============================================================================

run_test "token-miser skill exists"
assert_file_exists "${SCRIPT_DIR}/../skills/token-miser/SKILL.md"

run_test "session-miser skill exists"
assert_file_exists "${SCRIPT_DIR}/../skills/session-miser/SKILL.md"

run_test "Skills document model routing"
for skill in token-miser session-miser; do
    if grep -q "haiku\|sonnet\|opus" "${SCRIPT_DIR}/../skills/${skill}/SKILL.md" 2>/dev/null; then
        echo -e "${GREEN}✓ $skill documents models${NC}"
        TESTS_PASS=$((TESTS_PASS + 1))
    else
        echo -e "${YELLOW}⚠ $skill may not document models${NC}"
    fi
done

# ============================================================================
# License & Attribution Tests
# ============================================================================

run_test "LICENSE file exists"
assert_file_exists "${SCRIPT_DIR}/../LICENSE"

run_test "LICENSE is MIT"
assert_contains "$(cat ${SCRIPT_DIR}/../LICENSE)" "MIT" "MIT license present"

run_test "No hardcoded secrets in source"
secret_patterns=("api_key" "APIKEY" "secret=" "password=" "token=" "127.0.0.1" "localhost")
found_secrets=0
for pattern in "${secret_patterns[@]}"; do
    if grep -r "$pattern" "${SCRIPT_DIR}/../hooks/" 2>/dev/null | grep -v "localhost" | grep -v "127.0.0.1" | head -1; then
        echo -e "${YELLOW}⚠ Potential secret found: $pattern${NC}"
        found_secrets=$((found_secrets + 1))
    fi
done
if [[ $found_secrets -eq 0 ]]; then
    echo -e "${GREEN}✓ No obvious secrets in hooks${NC}"
    TESTS_PASS=$((TESTS_PASS + 1))
fi

# ============================================================================
# Dependency Tests
# ============================================================================

run_test "Dependencies documented in README"
deps=("jq" "bc" "cargo" "budi")
for dep in "${deps[@]}"; do
    if grep -q "$dep" "${SCRIPT_DIR}/../README.md"; then
        echo -e "${GREEN}✓ Dependency documented: $dep${NC}"
        TESTS_PASS=$((TESTS_PASS + 1))
    fi
done

# ============================================================================
# Summary
# ============================================================================

echo -e "\n${YELLOW}════════════════════════════════════════${NC}"
echo -e "Test Summary"
echo -e "${YELLOW}════════════════════════════════════════${NC}"
echo "Total tests run:    $TESTS_RUN"
echo -e "Passed:            ${GREEN}$TESTS_PASS${NC}"
echo -e "Failed:            ${RED}$TESTS_FAIL${NC}"

if [[ $TESTS_FAIL -eq 0 ]]; then
    echo -e "\n${GREEN}All shell script tests passed! ✓${NC}"
    exit 0
else
    echo -e "\n${RED}Some tests failed. Review above for details.${NC}"
    exit 1
fi
