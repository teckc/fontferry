# Code quality improvement record

## Baseline and scope

- Baseline: `3b32b54efb6dbb30865bbe1266ec4e1f3cfedc00` (latest `main` checked through GitHub on 2026-09-20).
- PRs #1–#17 were dependency updates, not fixes for these behaviors. No `AGENTS.md` was present. `CONTRIBUTING.md`, Cargo/pnpm configuration, CI and release workflows were read.
- Toolchain retained: Rust 1.97.1, Node 24, pnpm 11.9.0. No unrelated dependency upgrades. The final audit required rustls 0.23.42 → 0.23.45 (and its required webpki update), and sevenz-rust 0.6.1 → maintained sevenz-rust2 0.23.0; Cargo.lock records only this security-related graph change. The initial environment pnpm wrapper was 11.19.0; installation was repeated using `corepack pnpm` 11.9.0.
- Workspace: core owns policy/orchestration; platform owns SQLite/network/files/system APIs; Tauri provides CLI and commands; Svelte provides UI. CLI enters before the GUI single-instance plugin.
- Baseline core/platform: 10 tests passed. Baseline UI: one component test passed, typecheck and build passed. Baseline formatting passed. Existing main Actions runs report success, including [run 35055432589](https://github.com/teckc/fontferry/actions/runs/35055432589).

## Implemented changes

| Area | Verified problem and change |
| --- | --- |
| Ownership | Old installer unregistered/deleted all old paths after writing new paths. Use set differences, full SHA-256 names, reuse verified legacy owned paths, deduplicate identical content, and exclusive file creation. Unmanaged destination collisions are rejected. |
| Recovery | Old rollback removed current files before validating backup; uninstall ignored deletion failures. Persist an operation journal, preserve current files through SQLite commit, verify snapshots before restore, and keep failed cleanup in the journal. |
| Snapshots | Version-named backup directories could overwrite history. Use UUID directories and SHA-256 manifests. Legacy snapshot directories remain readable with mandatory font parsing; they lack historical integrity digests. |
| Concurrency | Instance-local mutex did not protect CLI against GUI. All engine mutations/recovery acquire a nonblocking OS file lock. Production lock is in the persistent data directory. Scheduled installation re-reads the installed variants/version under this lock. |
| Versions | Lexical comparison misordered two-part versions. Compare numeric two-/three-part tags (including padded parts and Adobe `R` suffix), SemVer prereleases and ISO date tags. Unknown labels return no ordering; release selection can use publication time, but automatic numeric update detection does not invent an order. Explicit selection applies the same channel/entitlement filters and still allows permitted downgrades. |
| Fingerprints | ETag/Last-Modified are change tokens: compare inequality, not numeric or lexical growth. |
| Releases | Fetch 100-item GitHub pages, stopping on a short page. Exceeding 100 pages is an explicit error rather than silent truncation. |
| Variants | UI restores installed choices and passes installed/status arrays explicitly into the details component so an already-open drawer updates after mutations. Nonempty catalogs reject empty selections and unknown IDs; the UI already disabled an empty selection, so backend behavior now agrees. Store sorted, deduplicated requested IDs. Legacy scheduled records with empty IDs resolve defaults once. |
| Local state | Cached statuses are rederived from current SQLite records without a network round trip. Cached remote information carries a check timestamp/source marker. Failed online checks report failure rather than claiming cached data is fresh. |
| Custom sources | Reject collisions with built-in IDs; existing conflicting persisted definitions cause a startup error rather than replacing built-ins silently. |
| Windows | Pair public-session Add/Remove flags, verify registry ownership, and restore session resources independently of registry presence. Drain interrupted-attempt resource references with a bounded loop and propagate registration/unregistration/registry/broadcast errors. Retain journal/files on unresolved failures. Platform behavioral limitations below remain mandatory acceptance work. |
| Scheduling | Propagate Windows/Linux disable failures, boot out macOS LaunchAgent before removing plist, check macOS replacement unload, reject unavailable systemd sessions rather than report an unimplemented fallback as enabled. Persist successful settings and restore the UI checkbox on failure. Escape systemd `%`, `$`, quotes and control characters. |
| Resource limits | ZIP/TAR/7z/raw fonts share an actual-output byte/entry budget. 7z uses an extraction callback with path checks before writes; the pinned dependency's default callback did not provide this boundary. Reject links/special TAR entries, duplicate output paths and cross-platform escape paths. Font parsing reads at most 256 MiB + one sentinel byte. Remote JSON/catalog/signature bodies are bounded while streaming. Download output names include an index to avoid sanitization collisions. |
| URL boundary | Validate typed IPv4/IPv6 hosts, mapped IPv4, credentials and redirects. A reqwest DNS resolver validates the addresses it actually returns to the connector, avoiding a separate preflight DNS check. Proxy caveat below applies. |
| Maintenance | Extract Tauri CLI/commands and Svelte font details/activity page; share operation types. Installer file/system work runs in `spawn_blocking`. Preserve install warnings/restart advice as activities. Failed scheduled checks record font ID/reason. |
| CI | Cargo checks/tests/build forwarding use `--locked`; dependency policy no longer regenerates Cargo.lock. Existing checks are retained. |

## Transaction and recovery invariants

1. Acquire the process lock **before** reading installation facts used for a mutation. Busy operations return a retryable human-readable error; process exit releases the OS lock. Do not unlink `operations.lock` while any FontFerry process is running.
2. Validate prepared bytes and ownership; create a uniquely named verified backup. Write and sync `font-operation.json` before installed-file mutations.
3. Copy only new paths, register additions, unregister removals and refresh. Keep old files on disk. Shared paths are never unregistered or deleted by the transition.
4. Commit the installed record in SQLite. SQLite is only the database commit authority, not a filesystem/system-API transaction.
5. Recovery compares the database version, owned-file set and snapshot identity with the journal. A committed operation finishes cleanup; an uncommitted operation restores old files/registration and removes new files. Uninstall commits when the DB record is absent.
6. Cleanup errors return failure and retain the journal. The DB may already describe the new installation; the journal explains remaining old files. Restart retries recovery before another mutation. A malformed journal or unavailable recovery material blocks mutation with an actionable error; it is never silently discarded.
7. Snapshots are removed only after state commit. A rollback creates a snapshot of the version being replaced, allowing a subsequent rollback in the opposite direction.

The journal covers process interruption. This implementation does **not** claim full power-loss atomicity across filesystem metadata, SQLite and platform APIs. Backup preparation can leave an unused UUID backup directory if interrupted before journal creation; it does not remove installed fonts. Automatic orphan-backup collection is intentionally not added without an age/ownership policy.

## Validation evidence

All executable changes at commit `44cf1effca05e2b505bb607d20fa4048f58df013` passed all five remote jobs: Rust on Windows 2025, macOS 15 and Ubuntu 24.04, Frontend (including three Playwright tests), and Dependency policy in [CI run 35516799357](https://github.com/teckc/fontferry/actions/runs/35516799357). The final record-only follow-up also removes one trailing blank line from a TypeScript type file; it makes no executable change. Native OS behavior still requires the acceptance steps below.

Local Linux checks:

- `cargo fmt --all -- --check`: passed after formatting.
- `cargo test -p fontferry-core -p fontferry-platform --locked -j 2`: passed: 27 tests (11 core, 16 platform), plus doc-test targets.
- `cargo clippy -p fontferry-core -p fontferry-platform --all-targets --locked -j 2 -- -D warnings`: passed, including the dependency repair.
- `corepack pnpm install --frozen-lockfile`: passed using pnpm 11.9.0.
- `corepack pnpm check`, `corepack pnpm test`, `corepack pnpm build`: passed; component tests increased from 1 to 4.
- `pnpm test:e2e`: three tests could not launch because Chromium was absent. `corepack pnpm exec playwright install chromium` retried and failed with HTTP 502/timeouts. No browser behavior is claimed validated locally. The remote Frontend job subsequently passed all three Playwright flows, component tests, check and build in [CI run 35516039203](https://github.com/teckc/fontferry/actions/runs/35516039203).
- Full workspace Clippy/tests could not finish locally: `pkg-config`/WebKitGTK development dependencies are absent. `apt-get` failed on container setgroups/setuid permissions. One initial concurrent build also encountered a zero-length intermediate object; isolated core/platform builds subsequently passed.
- `cargo xtask check` fails at workspace Clippy because pkg-config is absent. `cargo deny check` now passes with cargo-deny 0.20.2. Existing unused advisory-ignore warnings remain unchanged; no new ignore entries were added.

Regression tests use real temporary directories and SQLite, injected system registration/refresh/copy/delete failures, and independent OS processes. They cover repeated installation, overlapping payloads, partial copy/registration, unregister/refresh failure, SQLite commit failure, cleanup retry, missing/empty/corrupt rollback materials, version entitlement checks, dashboard derivation and fingerprint changes. Lock tests run a holder process, a competing process and another process after the holder is killed. Reconstructing an installer for journal recovery validates persistence, but is not a real power-loss experiment. No test operates on a real user font directory or scheduler.

## Remaining platform acceptance and limits

- **Windows:** use a disposable VM/account to install, update with a common file, uninstall and rollback while a second application enumerates/uses the font. Verify visibility before/after FontFerry exits and after sign-out/sign-in. Repeat with a font held open, injected registry denial and broadcast timeout. The old `FR_PRIVATE` installation may require sign-out before public registration is available. A crash between GDI and registry operations can require manual recovery; neither API provides a queryable global transaction, and a zero Remove result is reported as an error rather than assumed success. HKCU persistence and cross-session cleanup are not proven by Linux tests.
- **macOS/Linux/Windows scheduling:** test enable twice, disable twice, unload failures, a previously unloaded but still-present plist, and executable paths containing spaces/special characters in an isolated account. Removal queries the scheduler before deciding absence; actual repeated-operation behavior still requires the platform acceptance above. Persisted settings reflect the last successful FontFerry action, not continuous reconciliation with external scheduler edits. Linux StartupFallback remains unsupported and now reports failure honestly.
- **Network:** direct connections use validated resolver answers and redirect checks. A configured HTTP(S) proxy may resolve the origin itself; the application cannot enforce the proxy's upstream address selection. This is not a general network sandbox. Decoder header allocations/CPU and concurrent operations outside the font-install lock are not a hard process-memory sandbox.
- **Recovery:** valid legacy snapshots have no digest manifest; legacy integrity cannot be reconstructed retroactively. Actual valid-font rollback lifecycle and Windows cross-process registration need platform acceptance in addition to the synthetic-payload transition tests. Directory sync/power-cut testing, crash injection at every individual OS API instruction, and automatic orphan-backup collection remain outside verified coverage.
- **UI/Tauri:** Playwright remains blocked locally, and full Tauri compilation requires CI/system dependencies. The UI tests validate mocked IPC behavior; core/platform tests separately exercise actual Rust data/state behavior. SQLite calls remain synchronous behind a short mutex; large installer I/O and scheduler commands were the blocking paths moved off async workers.

Microsoft primary references: [AddFontResourceExW](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-addfontresourceexw), [RemoveFontResourceExW](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-removefontresourceexw). These describe private/public session behavior, registry persistence requirements, matching flags and in-use font limitations; they are not substitutes for per-user Windows acceptance.

## Audit-driven dependency repair

The first Draft PR CI audit failed on [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html) (rustls), [RUSTSEC-2026-0245](https://rustsec.org/advisories/RUSTSEC-2026-0245.html) and [RUSTSEC-2026-0246](https://rustsec.org/advisories/RUSTSEC-2026-0246.html) (sevenz-rust). These are relevant download/extraction dependencies, so they were repaired rather than ignored. The 7z callback still validates original entry paths and enforces the shared budget. A real, tiny 7z archive regression test now covers output limits, cumulative entry limits and path traversal with the replacement decoder. Production enables only the extraction utility feature; compression is a test-only dependency feature used to generate controlled archives.

## Copilot review follow-up

The follow-up merges main `05c54a7` (its dependency-update commits) into the PR without reverting those updates. It preserves the security decoder replacement and locked CI checks. TypeScript alone is restored to the previously validated 5.9.2: main's TypeScript 7.0.2 is explicitly rejected by svelte-check 4.7.6. The pnpm lockfile removes TypeScript 7's native binary packages accordingly. Core now declares the existing workspace tracing dependency to report diagnostic-storage errors.

| Review concern | Implemented response |
| --- | --- |
| Special-use IPv4 | Reject all of 192.0.0.0/24 consistently for literal URLs, mapped IPv6 and the DNS-address predicate. This is a conservative application policy, including the .9/.10 anycast exceptions, not a claim that the entire range is RFC1918 space. See the [IANA registry](https://www.iana.org/assignments/iana-ipv4-special-registry/). Tests cover every address in the range. |
| Replaced obsolete files | Preflight old/target journal hashes and file types before any recovery mutations; recheck before deleting. Replaced files and symlinks retain the journal and return an actionable error. Both commit and compensation directions are tested. |
| Recovery preflight | The existing code already validated required backups in a separate pass. It now also validates all destinations and obsolete paths before copying. Tests cover a later corrupt backup, later conflicting destination and dangling symlink. |
| Invalid journals | Validate file-set/record consistency, shared-path hashes, install/uninstall shape, hash syntax, font-directory containment and snapshot paths before recovery mutation. Unknown database states block recovery. This is consistency validation, not authentication against a malicious local account. |
| Activity failures | Diagnostic persistence cannot change successful install/uninstall/rollback results or replace the primary failure. Tracing reports activity-storage failures. A scheduled re-check failure records its font ID and reason. |
| Cache message | The warning accurately says metadata is retained for provenance and online freshness is unconfirmed. Network failures still return errors. |
| Version ordering | Decide semantic-version vs publication-time ordering once for the entire eligible set, avoiding non-transitive pairwise fallback. All six permutations of a mixed-label example select the same release. Opaque tag text is only a deterministic equal-timestamp tie-break. |
| Legacy variants | Empty legacy records resolve defaults when opening details; a subsequent explicit deselection remains empty and disables installation. Nonempty saved choices remain authoritative. |
| Scheduler serialization/state | GUI and CLI use one helper and the same OS operation lock as font mutation. A SQLite pending intent is saved before native commands. Any partial command/persistence failure leaves a durable unknown state; settings show that state and allow retry. Success clears the intent only after saving the new state. Tests use real SQLite write-failure triggers and reopen the database. No destructive native scheduler command is run by these tests. |
| Windows retries | Unregister establishes a known public-session reference before draining it, so an earlier successful GDI removal followed by registry failure does not permanently poison retries. Zero removal after a known add remains an error. Legacy resources in the current process are also drained with matching FR_PRIVATE flags. Injected reference-count tests verify orchestration only; cross-process/font-in-use/sign-in acceptance remains necessary. |

### Conservative recovery of incomplete copies

A failed direct copy can leave bytes that do not match the intended final hash. Such bytes cannot safely be distinguished from an externally replaced file after a crash. Recovery now **preserves the mismatching file and journal**, returns its path, and blocks further font mutation rather than deleting it by assumption. Complete matching additions can still be compensated automatically. This deliberately replaces the earlier test expectation that any partial destination is silently removed.

For manual recovery: close FontFerry/font-using applications, preserve the database, journal and backup directories, inspect the reported path and journal hashes, and move a confirmed conflicting/incomplete file to a separate quarantine location without discarding it. Retry recovery only once the affected paths and recovery material are understood. Do not delete the journal to bypass a failed integrity check. Hash preflight does not provide protection against an adversarial concurrent filesystem writer; the process lock only serializes FontFerry.

### Follow-up verification

- Local Linux: 39 Rust tests passed (14 core, 25 platform), including temporary-directory/SQLite failure recovery and independent-process locking. The Unix-only dangling-symlink test is not a Windows test.
- Core/platform all-target Clippy with `--locked -- -D warnings`, formatting and `cargo deny check`: passed; no new advisory ignore.
- pnpm 11.9.0 frozen installation, Svelte check, 6 component tests and production build: passed after the TypeScript compatibility repair.
- The local default build encountered zero-length object/linker failures. Tests passed in a fresh target directory with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_CODEGEN_UNITS=1`; the remote CI retains the standard compiler configuration and required gates.
- Full local Tauri/workspace checks still require unavailable pkg-config/WebKitGTK development dependencies. Local Playwright initially could not launch its absent Chromium; browser installation is retried separately. No local native Windows/macOS behavior is claimed.
- Updated remote CI evidence is recorded in the PR after pushing this follow-up; earlier successful runs are not evidence for these new changes.

Scheduler intent records expose uncertainty rather than attempt an unsafe automatic task rollback. An explicit retry reapplies the selected target idempotently. External scheduler edits remain outside continuous reconciliation. Neither this change nor the font journal claims power-loss atomicity or native-platform acceptance from mock tests.
