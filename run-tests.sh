#!/usr/bin/env bash
# Unified test runner for CostGuard test suite
# Runs integration tests, unit tests, and shell script tests
# Usage: ./run-tests.sh [--integration|--units|--hooks|--all]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

# Defaults
TEST_TYPE="${1:-all}"
VERBOSE=false
JOBS=4

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --integration) TEST_TYPE="integration" ;;
        --units) TEST_TYPE="units" ;;
        --hooks) TEST_TYPE="hooks" ;;
        --all) TEST_TYPE="all" ;;
        --verbose) VERBOSE=true ;;
        -j|--jobs) JOBS=$2; shift ;;
        -h|--help)
            echo "Usage: $0 [--integration|--units|--hooks|--all] [--verbose] [-j JOBS]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
    shift
done

# Reporting
start_section() {
    echo -e "\n${YELLOW}════════════════════════════════════════${NC}"
    echo -e "${YELLOW}$1${NC}"
    echo -e "${YELLOW}════════════════════════════════════════${NC}\n"
}

pass_test() {
    echo -e "${GREEN}✓ $1${NC}"
}

fail_test() {
    echo -e "${RED}✗ $1${NC}"
    return 1
}

total_passed=0
total_failed=0

# ============================================================================
# Integration Tests
# ============================================================================

run_integration_tests() {
    start_section "Integration Tests"

    if [[ ! -d "analytics/costguard-pulse" ]]; then
        fail_test "costguard-pulse directory not found"
        return 1
    fi

    cd analytics/costguard-pulse

    if [[ "$VERBOSE" == "true" ]]; then
        cargo test --test integration -- --nocapture 2>&1
    else
        cargo test --test integration --quiet 2>&1
    fi

    if [[ $? -eq 0 ]]; then
        pass_test "Integration tests (45+)"
        total_passed=$((total_passed + 1))
    else
        fail_test "Integration tests"
        total_failed=$((total_failed + 1))
        return 1
    fi

    cd - >/dev/null
}

# ============================================================================
# Unit Tests
# ============================================================================

run_unit_tests() {
    start_section "Unit Tests"

    cd analytics/costguard-pulse

    if [[ "$VERBOSE" == "true" ]]; then
        cargo test --test unit_tests -- --nocapture 2>&1
    else
        cargo test --test unit_tests --quiet 2>&1
    fi

    if [[ $? -eq 0 ]]; then
        pass_test "Unit tests (35+)"
        total_passed=$((total_passed + 1))
    else
        fail_test "Unit tests"
        total_failed=$((total_failed + 1))
        return 1
    fi

    cd - >/dev/null
}

# ============================================================================
# Hook Tests
# ============================================================================

run_hook_tests() {
    start_section "Shell Hook Tests"

    if [[ ! -f "hooks/test-hooks.sh" ]]; then
        fail_test "hooks/test-hooks.sh not found"
        return 1
    fi

    if bash hooks/test-hooks.sh; then
        pass_test "Shell hook tests"
        total_passed=$((total_passed + 1))
    else
        fail_test "Shell hook tests"
        total_failed=$((total_failed + 1))
        return 1
    fi
}

# ============================================================================
# Main Execution
# ============================================================================

case "$TEST_TYPE" in
    integration)
        run_integration_tests || exit 1
        ;;
    units)
        run_unit_tests || exit 1
        ;;
    hooks)
        run_hook_tests || exit 1
        ;;
    all)
        run_integration_tests || true
        run_unit_tests || true
        run_hook_tests || true
        ;;
    *)
        echo "Unknown test type: $TEST_TYPE"
        exit 1
        ;;
esac

# Summary
start_section "Test Summary"
echo "Test suites passed:  ${GREEN}$total_passed${NC}"
echo "Test suites failed:  ${RED}$total_failed${NC}"

if [[ $total_failed -eq 0 ]]; then
    echo -e "\n${GREEN}All test suites passed! ✓${NC}"
    exit 0
else
    echo -e "\n${RED}Some test suites failed. Review above for details.${NC}"
    exit 1
fi
