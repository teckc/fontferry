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
