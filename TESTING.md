# CostGuard Testing Guide

## Quick Start

Run all tests:

```bash
./run-tests.sh --all
```

Run specific test suite:

```bash
./run-tests.sh --integration  # 45+ integration tests
./run-tests.sh --units        # 35+ unit tests
./run-tests.sh --hooks        # Shell script tests
```

Run with verbose output:

```bash
./run-tests.sh --all --verbose
```

---

## Test Structure

### 1. Integration Tests (45+)

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

### 2. Unit Tests (35+)

**Location**: `analytics/costguard-pulse/tests/unit_tests.rs`

Tests isolated functions:
- Token parsing and formatting
- Cost calculations
- Timestamp handling
- Buffer overflow prevention
- SQL injection prevention

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

| Area | Tests | Coverage |
|------|-------|----------|
| Session management | 8 | 90% |
| Transcript parsing | 6 | 95% |
| Cost calculations | 9 | 92% |
| Reporting | 10 | 88% |
| Budget/calibration | 8 | 85% |
| Edge cases | 15 | 80% |
| Error handling | 10 | 85% |
| **Total** | **70+** | **~89%** |

---

## Performance

All 70+ tests complete in **60-90 seconds** on modern hardware.

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

### GitHub Actions

CI automatically runs all tests on:
- Every push to any branch
- Every pull request

**Required for merge**:
- All integration tests pass
- All unit tests pass
- Code coverage ≥85%

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

See `TEST_SUITE.md` for detailed documentation of all 70+ tests.
