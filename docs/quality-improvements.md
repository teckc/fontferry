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
| Windows | Pair public-session Add/Remove flags, verify registry ownership, propagate registration/unregistration/registry/broadcast errors. Retain journal/files on unresolved failures. Platform behavioral limitations below remain mandatory acceptance work. |
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
