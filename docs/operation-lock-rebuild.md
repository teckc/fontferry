# Explicit operation-lock release

This patch was reconstructed from main `ce5e47c` after the earlier private
checkout became unavailable. It is not claimed to reproduce an earlier Git SHA.

The old operation guard was a bare `File`. On Linux, `flock` belongs to an open
file description. A child created by another thread can briefly inherit a
reference before exec, so closing the parent's descriptor does not necessarily
release the lock immediately. An immediate new operation can incorrectly see a
busy lock even though the preceding operation has ended.

`OperationGuard` now owns a private descriptor and the acquiring process ID. Drop
explicitly unlocks before closing, only when executed by the acquiring process.
The public guard has no clone/descriptor accessor. A forked child's copy must not
unlock the parent's active operation. The advisory lock file is never unlinked.
Errors from unlocking in Drop cannot be returned; closing remains the fallback.
This does not make the font operations themselves transactional.

Validation is deliberately separated:

- Rust tests check contention, release while a real child process remains alive,
  abnormal process exit, a surviving descriptor alias, and the non-owner PID
  branch. The alias/PID tests are deterministic models, not unsafe Rust fork tests.
- `python3 scripts/test-operation-lock-fork.py` demonstrates the actual Linux
  fork/flock mechanism using temporary files: close-only remains blocked while
  the child retains its descriptor; explicit unlock permits a new acquisition.
  The Python test does not call the Rust guard and does not establish Windows or
  macOS kernel behavior.
- Cross-platform Rust execution remains a CI acceptance requirement.
