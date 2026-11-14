# Windows alpha.5 → candidate installer compatibility

Status: static review only. No installation, uninstallation, scheduled-task mutation or real font operations were performed. Baseline: v0.2.0-alpha.5; candidate branch: fix/tls-installer-font-management.

## Findings and correction

Both baseline and candidate use tauri CLI 2.11.4 in pnpm-lock.yaml. The exact upstream tag tauri-cli-v2.11.4 was read, including nsis/mod.rs and nsis/installer.nsi.

- Product: FontFerry; bundle identifier: io.github.teckc.fontferry; binary: fontferry; install mode: currentUser. All preserved.
- Default Windows install location: LOCALAPPDATA/FontFerry. Uninstall registry key: HKCU/Software/Microsoft/Windows/CurrentVersion/Uninstall/FontFerry.
- IMPORTANT: NSIS manufacturer defaults to the second identifier segment, github. It is used in Software/github/FontFerry to remember the selected installation location. The newly added publisher=teckc would have changed that lookup to Software/teckc/FontFerry. It has been removed before shipping. Do not add publisher branding without a tested registry/path migration.
- No custom NSIS template, installation hooks, shortcut folder, binary name, identifier or per-machine installation mode is introduced. Added installer languages/icon do not intentionally change identity.
- Two static regression tests pin identity and the previous-location key; all six release/installer tests pass. They do not replace installed-binary testing.

## Data, fonts and uninstall behavior

Application data paths in paths.rs are unchanged from alpha.5. ProjectDirs uses organization teckc and application FontFerry. On Windows this maps local state to LOCALAPPDATA/teckc/FontFerry/data (fontferry.db, logs, backups) and cache to LOCALAPPDATA/teckc/FontFerry/cache. These are separate from the executable installation directory.

The standard Tauri uninstaller offers an explicit delete-app-data checkbox and skips that deletion in update mode. However, its deletion targets APPDATA/io.github.teckc.fontferry and LOCALAPPDATA/io.github.teckc.fontferry, NOT the above ProjectDirs state. Therefore that checkbox must not be advertised as removing the font ledger, backups or all settings. Existing behavior retains those files even if the generic checkbox is selected.

Managed fonts are installed in LOCALAPPDATA/Microsoft/Windows/Fonts and registered only under HKCU/Software/Microsoft/Windows NT/CurrentVersion/Fonts. No NSIS hooks remove them. Ordinary application uninstall is thus expected to retain managed fonts and their ledger. This is separate from the app's explicit per-font uninstall command.

An existing daily task FontFerry/UserDailyUpdate references the executable path. No NSIS hook removes it on application uninstall. A dangling task is a pre-existing cleanup gap. Do not silently delete the ledger while leaving managed fonts or a task without recoverable ownership records.

## Disposable Windows account / VM test plan (NOT executed)

Use a snapshotted Windows VM with a dedicated disposable local standard user, not the owner's working account. Test artifacts must be built from the named refs and hashes recorded. The candidate must have a deliberately higher test version so this is an upgrade, not an equal-version reinstall. Do not publish the test tag or change production update channels.

1. Baseline: install alpha.5 in default location. Record HKCU uninstall entry, Software/github/FontFerry, shortcut targets, process path and file hashes. Confirm exactly one uninstall entry.
2. Seed state: add a harmless source and manual-version record. Install one freely licensed test font only in this disposable account, keep one unrelated user-installed font as a control, and create a rollback snapshot. Record ledger, font filenames/registry entries and hashes. Enable daily update only in this isolated account and record task action.
3. Default-path upgrade: run candidate installer with app closed. Check install destination and uninstall identity unchanged; one entry and no duplicate executable trees. Confirm source/settings/manual version, managed font count, files, backup/rollback and task action retained.
4. Custom-path upgrade: revert snapshot, install alpha.5 to a path with spaces/non-ASCII characters, then upgrade. Confirm the stored custom path is reused; no fallback second copy under LOCALAPPDATA/FontFerry. This specifically exercises the publisher regression.
5. Running-app upgrade: start alpha.5, upgrade through ordinary installer and then a separate signed-updater test channel. Confirm process-close handling, no duplicate running process, no partial replacement, preserved state and restart. Never target the production channel.
6. Language matrix: English, Simplified Chinese and Traditional Chinese. Check readable install/uninstall pages and remembered language without changing path or identity. Run as non-admin and verify no new elevation requirement.
7. Uninstall KEEP settings: in a disposable snapshot, leave delete-data unchecked. Verify executable/shortcuts/uninstall entry removed, ledger/backups and both managed/unrelated font files retained. Record the existing daily-task residue as a known defect, not a passing cleanup result. Reinstall and confirm ownership/rollback state recovers.
8. Uninstall DELETE settings: separate snapshot with checkbox selected. Record exactly which identifier and ProjectDirs directories survive. Current expectation is ProjectDirs state remains; this is a product-semantic gap to resolve, not evidence that complete deletion works. Do not expect the unrelated font to be touched.
9. Failure/cancel cases: cancel installer before installation, cancel uninstall, reject running-app closure, simulate failed upgrade in VM snapshot. Existing app and ledger must remain usable; record event logs and installer output.
10. Teardown: restore/delete the disposable VM or account only after recording evidence, never by deleting the owner's font folders or task.

## Follow-up before release

- Decide and label app uninstall versus explicit font removal separately. Keep fonts by default.
- Provide a deliberate, separately confirmed settings cleanup that matches ProjectDirs, considers backups/ownership, and does not delete unrelated fonts.
- Remove only this installation's daily task on true uninstall (not upgrade); verify task executable matches the installation before mutation.
- Complete MSVC build, fresh install, upgrade and uninstall matrix. No installer compatibility sign-off until these pass.

Sources: https://github.com/tauri-apps/tauri/tree/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis ; project paths.rs/install.rs/schedule.rs; directories-rs Windows path construction (https://github.com/dirs-dev/directories-rs/blob/master/src/win.rs).

## Ownership and interrupted recovery review (2026-11-14)

Reviewed on commit 9a95a83; this section changes documentation only. Source evidence: state.rs (WAL and installed_fonts), install.rs::backup/begin/recover/resolve/uninstall, engine.rs::uninstall/uninstall_inner, commands.rs::dashboard and cli.rs::run_cli. No live database was opened and no font operation was run.

### Four separate objects, not one delete-settings switch

| Object | Examples | Safe intended treatment |
| --- | --- | --- |
| Application | executable, shortcuts, uninstall registration | Ordinary app removal does not imply font removal or ledger reset. |
| Managed fonts | owned_files and their Windows font registrations | Remove only through ownership-aware font operations; never clear the whole user Fonts directory. |
| Management state | fontferry.db, source/settings/license/ownership records, previous snapshot references | Preserve when retaining fonts. It is operational state, not disposable preferences. |
| Unfinished recovery set | font-operation.json, coherent database state, backup files + manifest.json, affected font files | Preserve together until recovery is resolved and verified. Do not treat these as cache. |

The SQLite installed_fonts JSON record carries owned_files, version/variants and previous snapshot reference. A backup contains copies of the previously owned fonts plus manifest.json with hashes. The operation journal records old state, old hashes, target hashes and the backup. resolve() verifies the recovery plan and file hashes before mutation and removes the journal only after successful finalization. GUI dashboard and CLI startup call recovery, so a later ordinary launch can act on retained recovery state.

### Concrete loss-of-state failure modes

1. Delete the database but keep installed fonts, with no journal: the application loses its ownership/version/snapshot linkage. A generic filesystem scan is not an equivalent reconstruction of the ledger. Do not silently adopt or remove those files by guessed filenames.
2. Delete the database but retain an unfinished uninstall journal: recover() treats `(operation.version=None, installed=None)` as committed. A newly recreated empty database can therefore make a previously uncommitted uninstall look committed, and resolve() can finish removing the journal's old font files. This is a code-path inference, NOT an executed reproduction. It is a reason to forbid partial cleanup, not to rely on recovery failing closed in every missing-database case.
3. Delete the database but retain an update journal: the current DB may match neither the old nor new record; recovery reports manual repair. Losing the ledger is not repaired by retaining only font files.
4. Delete the journal while a font operation is unfinished: the automatic recovery trigger and operation context disappear; stale files/registrations can remain while the ledger reflects a different phase.
5. Delete a referenced snapshot: rollback or interrupted-operation recovery can no longer restore verified old bytes. The current resolver reports missing/corrupt backup and preserves evidence rather than inventing data.
6. Copy only fontferry.db while a writer is live: SQLite is configured for WAL; committed transactions may still reside in its WAL. Use a verified consistent SQLite backup or stop all writers and capture a coherent state set. Do not delete a WAL or SHM file as generic cleanup while the database is active.

### Additional isolated test cases (NOT executed)

Add fault-injection cases to the mock FontSystem harness, then run equivalent lifecycle tests only in a disposable VM/account. Capture state before/after each boundary; do not fabricate a successful recovery result.

- Uninstall interrupted after journal persistence/unregistration but before SQLite record deletion: restart must preserve/restore the old installation and registrations, then finish recovery consistently.
- Uninstall interrupted after SQLite record deletion but before finish(): restart must complete only the selected font's removal and clear the journal; an unrelated control font remains unchanged.
- Install/update interrupted before and after SQLite commit: verify old-versus-new state selection, hash checks, registration state and retained rollback snapshot.
- Missing database + retained uninstall journal: explicitly reproduce the misclassification above using temp paths/mock fonts. Treat destructive cleanup of the database in this state as disallowed product behavior, even if the current resolver follows its existing committed branch.
- Missing/corrupt database, malformed journal, missing/corrupt snapshot and externally modified font: retain the evidence set and expose a repair path; do not add automatic broad cleanup.
- App uninstall/reinstall while a valid recovery journal exists: recovery data and the font set must survive app removal unchanged. Reinstalled app must resolve from coherent retained state, not from an empty replacement ledger.
- Settings-only reset with installed fonts: ownership, rollback linkage, license records required for operation, and unfinished recovery must not be erased. A future preferences reset needs selective settings handling, not database deletion.
- Intentional complete removal: first resolve unfinished recovery, then explicitly remove selected managed fonts through the engine, verify no pending journal/remaining ownership, and only then separately confirm management-data cleanup. Keep unrelated fonts untouched. If the user intends to keep fonts, retain coherent management state rather than calling its destruction a harmless reset.
- Scheduled/headless writer during cleanup: prevent races by stopping the installation-owned task/writer and acquiring the same operation lock before any future cleanup. A running GUI is not the only possible writer.

Release gate: no new data-clearing hook or reset feature until these invariants are implemented and tested. The current generic NSIS data checkbox mismatch remains a separate UX defect; simply changing its deletion path to ProjectDirs would introduce a more dangerous state-loss bug.

## Existing alpha updater reachability (2026-11-14 UTC)

Read-only HTTP checks and source comparison, not an installed-client execution:

- GitHub releases/latest redirects to v0.1.0. Its expanded assets list contains FontFerry-osx-arm64.tar.gz, FontFerry-osx-x64.tar.gz and FontFerry-win-x64.zip, no latest.json. GET releases/latest/download/latest.json returned HTTP 404.
- GET releases/download/beta/latest.json returned HTTP 200, version 0.2.0-alpha.5, keys linux-x86_64, darwin-universal, windows-x86_64. The nonstandard macOS key is already corrected by this branch's generator for future releases; no remote asset was modified.
- alpha.5 tauri.conf.json defaults to the stable latest endpoint, BUT its src/lib.rs::update_endpoint already offers stable and beta, and its settings UI lets the user select beta (default stable). The current refactored commands.rs preserves these endpoints.
- The current candidate has the same updater public key as alpha.5. Equality does not prove that a new artifact was correctly signed or that its private signing key is available. No private key was accessed.

Interpretation: the stable metadata endpoint is currently unavailable for this Rust alpha's update flow. It is incorrect to infer that all old alpha clients have no upgrade route: the existing beta selector supplies another route. It currently advertises the same version, not a newer security-fixed build. No exact old-client error screen, signature verification or installation outcome has been tested.

### Signed old-client upgrade and direct-installer fallback

1. After explicit release approval, build a higher candidate version, using the existing installer identity and the signing key corresponding to alpha.5's embedded public verifier. Do not rotate keys casually, disable signature checks, or rename a prerelease as stable just to make GitHub latest resolve.
2. Publish immutable versioned artifacts completely before updating the opt-in beta manifest; manifest URLs should refer to that immutable release. Check HTTP status, four supported OS-ARCH keys, version monotonicity and downloadable signature/artifact pairs. Re-fetch the beta endpoint after publication.
3. In a disposable Windows account, install the real alpha.5 package and use its settings to select beta. Verify detection, download, actual signature acceptance and upgrade with retained state. Also test tampered artifact/signature rejection in an isolated test channel; never tamper production assets. macOS Intel and Apple Silicon need separate coverage.
4. If the old client cannot use that route, provide the named, versioned Windows NSIS installer as the fallback, not the repository's generic latest link pointing to v0.1.0. Verify provenance/checksum and platform signing state. Test an in-place install over alpha.5; do not instruct users to clear their font ledger or uninstall first.
5. A future stable Rust release can provide a correct manifest at stable latest after its normal release checks. Do not attach misleading alpha metadata to the unrelated v0.1.0 release as an unreviewed shortcut.

Local tests added: separate stable/beta contract and alpha.5 public-key continuity. Together with installer/manifest tests, 8 tests pass. These static assertions are explicitly not signature verification or client update tests.
