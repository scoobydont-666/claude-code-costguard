# CostGuard Testing Guide

## Honest Current State (2026-04-18)

- **Integration tests** (`tests/integration.rs`): 57 defined, **55 pass, 2 fail**.
  The 2 failing tests assume an isolated fixture directory; they regress when
  run in an env that has real Claude transcript data (they try to discover a
  "no transcripts" baseline and find real ones). Fix requires switching to
  tmpdir-rooted fixtures. Tracked as a `known_failing` category below.
- **Unit tests** (`tests/unit_tests.rs`): 25 defined — most are **documentation
  placeholders** (trivial `assert_eq!(x, x)` to describe expected behavior).
  They pass but do not exercise real parsing/calculation paths.
- **Hook tests** (`hooks/test-hooks.sh`): shell-only static checks. No CI.
- **CI**: `.github/workflows/security.yml` runs gitleaks only. Test runs are
  not wired into CI.

So the earlier "70+ tests" / "~89% coverage" table was aspirational-tense
prose, not measurements. The real shipping invariants are: 55 integration
tests pass, and the binaries build cleanly.

---

## Quick Start

Run all tests:

```bash
./run-tests.sh --all
```

Run specific test suite:

```bash
./run-tests.sh --integration  # 57 tests (55 pass, 2 need isolated fixtures)
./run-tests.sh --units        # 25 placeholder tests (docs-as-tests)
./run-tests.sh --hooks        # Shell script tests (static checks)
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

**Categories**:
- **Phase 1** (10 tests): Core session/hook functionality
- **Phase 2** (7 tests): Data integrity and sync
- **Phase 3** (10 tests): Reporting and display
- **Phase 4** (18 tests): Advanced features and edge cases

### 2. Unit Tests (25, placeholder)

**Location**: `analytics/costguard-pulse/tests/unit_tests.rs`

Intended to test isolated functions:
- Token parsing and formatting
- Cost calculations
- Timestamp handling
- Buffer overflow prevention
- SQL injection prevention

**Current reality:** these 25 tests describe expected behavior in comments
but assert tautologies (e.g. `assert_eq!(500_000_000, 500_000_000)`). They
exist to document the parse/calc contract and should be rewritten to call
the actual functions once those are exposed from `main.rs`.

**Run**:
```bash
cd analytics/costguard-pulse
cargo test --test unit_tests
```

**Coverage**:
- Parsing (5 tests)
- Calculations (5 tests)
- Data integrity (5 tests)
- Error handling (5 tests)
- Edge cases (5 tests)
- Configuration (5 tests)

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
| Integration (`tests/integration.rs`) | 57 | 55 | 2 fail when real transcripts exist in $XDG_DATA_HOME |
| Unit (`tests/unit_tests.rs`) | 25 | 25 | Placeholders — tautological asserts |
| Hooks (`hooks/test-hooks.sh`) | N/A | — | Static checks, no assertions beyond existence/executability |
| **Total** | **82** | **80** | 2 flakes on real-data envs |

---

## Performance

The 80 currently-passing tests complete in about 180 seconds on an i5-8500
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

See `TEST_SUITE.md` for detailed per-test documentation (82 tests defined).
