# CostGuard Testing Guide

## Current State (2026-04-18)

- **Integration tests** (`tests/integration.rs`): **55 tests, all passing**.
  Exercise the `costguard-pulse` CLI and the `costguard-pulse-hook` binary
  against a real filesystem + SQLite DB, with each test using a unique
  `XDG_DATA_HOME` for isolation.
- **Hook tests** (`hooks/test-hooks.sh`): shell-only static checks
  (existence, permissions, license headers). No CI wiring.
- **CI**: `.github/workflows/security.yml` runs gitleaks only. No
  test-runner workflow (October 2026 scale-back — test-runner was
  costing Actions minutes without catching anything the author wasn't
  already running locally).

The older "70+ tests / ~89% coverage" language in this file was design
intent, never measurement. It's gone. The unit-tests suite that held
25 tautological placeholders (`assert_eq!(x, x)` describing expected
parse/calc behavior in comments) was deleted 2026-04-18; the
integration tests already exercise the same parse/calc code paths
through the CLI, so the placeholders were dead weight.

---

## Quick Start

Run all tests:

```bash
./run-tests.sh --all
```

Run specific test suite:

```bash
./run-tests.sh --integration  # 55 tests (end-to-end CLI + hook)
./run-tests.sh --hooks        # Shell script static checks
```

Run with verbose output:

```bash
./run-tests.sh --all --verbose
```

---

## Test Structure

### 1. Integration Tests (57)

**Location**: `analytics/costguard-pulse/tests/integration.rs`

Tests the costguard-pulse CLI and hook binaries with:
- Real filesystem operations
- SQLite database
- Actual transcript parsing
- End-to-end workflows

**Run**: 
```bash
cd analytics/costguard-pulse
cargo test --test integration
```

**Categories** (approximate, by test name prefix):
- Core session/hook functionality (~10 tests)
- Data integrity and sync (~7 tests)
- Reporting and display (~10 tests)
- Fleet, efficiency, budgets, anomalies (~18 tests)
- Robustness / edge cases (~10 tests)

Exact split fluctuates as tests are added/removed. Count of `#[test]`
attributes in `tests/integration.rs` is the source of truth.

### 2. Unit Tests — deleted 2026-04-18

`tests/unit_tests.rs` previously held 25 tautological placeholders
(`assert_eq!(x, x)` describing expected parse/calc behavior in
comments). The integration suite already exercises those code paths
through the CLI, so the placeholders were dead weight. File removed.

If you need real unit coverage of parse/calc, expose the functions
from `main.rs` into a `lib.rs` + add tests that call them directly.

### 3. Shell Hook Tests

**Location**: `hooks/test-hooks.sh`

Validates:
- Hook script existence and executability
- Configuration file validity
- Documentation completeness
- License and attribution
- Dependency requirements

**Run**:
```bash
bash hooks/test-hooks.sh
```

---

## Test Coverage

The prior table here claimed measured coverage (~89%). CostGuard does not
currently have `cargo-tarpaulin` or `cargo-llvm-cov` wired, so any coverage
number was unsubstantiated. Raw test counts per suite:

| Suite | Count | Passing | Notes |
|-------|-------|---------|-------|
| Integration (`tests/integration.rs`) | 55 | 55 | End-to-end CLI + hook; unique XDG_DATA_HOME per test |
| Hooks (`hooks/test-hooks.sh`) | N/A | — | Static checks, no assertions beyond existence/executability |
| **Total** | **55** | **55** | |

---

## Performance

All 55 integration tests complete in under 180 seconds
(parallel execution enabled).

### Test Timing

| Test | Duration |
|------|----------|
| Single core CLI test | 5-10ms |
| Transcript parsing | 50-100ms |
| All integration tests | 45-60s (parallel) |
| All unit tests | 1-5s |
| All hook tests | 5-10s |

---

## Critical Test Scenarios

### 1. Cost Accuracy

Validates that token usage correctly converts to USD:

```
Haiku (100 input, 50 output):
  (100 × $1 + 50 × $5) / 1M = $0.00035

Opus (100 input, 50 output):
  (100 × $5 + 50 × $25) / 1M = $0.00175

Ratio: ~5.0x (correct) ✓
```

### 2. Cache Hit Calculation

Validates cache read percentage:

```
cache_read = 1000
input + cache_read + cache_write = 1300
hit% = 1000/1300 × 100 = 76.9% ✓
```

### 3. Live Session Token Integration

Validates that statusline shows tokens from active (unsync'd) transcripts.

### 4. Robustness

Tests handle:
- Malformed JSON
- Missing fields
- Special characters in paths
- Very large session IDs (256+ chars)
- Zero token usage
- Unknown model names

---

## Continuous Integration

### GitHub Actions (current)

`.github/workflows/security.yml` runs **gitleaks only**. There is no
test-runner workflow yet. The claims below are a ROADMAP, not shipping
state — see the "Honest Current State" section at the top of this file.

### Test-runner CI (planned)

CI should run all tests on:
- Every push to any branch
- Every pull request

Intended merge gates:
- All integration tests pass (57/57 — requires tmpdir fixture fix first)
- All unit tests pass AND exercise real functions (not placeholders)
- Code coverage measured via `cargo-llvm-cov`, not hand-waved

### Local Pre-Commit

To run tests before commit:

```bash
# Add to .git/hooks/pre-commit
#!/bin/bash
./run-tests.sh --all || exit 1
```

---

## Debugging Failed Tests

### Enable Verbose Output

```bash
./run-tests.sh --integration --verbose
```

### Check Specific Test

```bash
cd analytics/costguard-pulse
cargo test --test integration test_name -- --nocapture
```

### Inspect Test Database

Failed tests leave their database in `/tmp/costguard-pulse-test-{id}/`:

```bash
sqlite3 /tmp/costguard-pulse-test-*/costguard-pulse/pulse.db
```

### Review Hook Errors

Hooks log to stderr; capture full output:

```bash
cargo test --test integration test_name 2>&1 | grep -i error
```

---

## Adding New Tests

### Integration Test Template

```rust
#[test]
fn test_my_feature() {
    let dir = unique_data_dir();
    
    // Setup
    run_hook(&dir, "session-start", r#"{"sessionId":"s1"}"#);
    
    // Action
    let result = run_cli(&dir, &["command"]);
    
    // Assert
    assert!(result.contains("expected"));
}
```

### Unit Test Template

```rust
#[test]
fn test_my_calculation() {
    let input = 100;
    let result = calculate(input);
    assert_eq!(result, 200);
}
```

---

## Test Data

### Test Transcript Format

Standard JSONL with message types:

```json
{"type":"progress","timestamp":"2026-03-20T10:00:00.000Z","sessionId":"sess-1"}
{"type":"assistant","timestamp":"2026-03-20T10:00:01.000Z","message":{"model":"claude-opus-4-6","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200}}}
{"type":"user","timestamp":"2026-03-20T10:00:10.000Z","uuid":"user-1"}
```

### Test Isolation

Each test gets unique `XDG_DATA_HOME=/tmp/costguard-pulse-test-{id}/`

Parallel tests don't interfere.

---

## Known Limitations

1. **Network Tests**: `sync-remote` uses mocked SSH, no real network
2. **Timer Tests**: `install-timer` creates files but doesn't run systemd
3. **Stress Tests**: Tested up to 5-10 sessions; not benchmarked at 1000+
4. **Large Files**: Transcripts tested up to ~100 lines; not 100k+ lines

---

## Maintenance Schedule

- **Monthly**: Verify model pricing, check for deprecated CLI commands
- **Per Release**: Add tests for new features
- **Per Bug Fix**: Add regression test

---

## License

Test suite is MIT licensed (same as CostGuard).

---

## Questions?

See `TEST_SUITE.md` for per-test documentation (55 tests defined).
