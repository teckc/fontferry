# Desktop hardening and release work

## Implemented

- Updated only the four TLS trust-chain packages in Cargo.lock using cargo update: rustls 0.23.46, rustls-webpki 0.103.16, webpki-roots and webpki-root-certs 1.0.10. No advisory suppression added.
- Split RustSec audit from cargo-deny so one failed policy check cannot hide the other report. Tag builds now require the same frontend, three-platform Rust and policy checks as pull requests.
- Replaced the inline updater manifest generator with a tested, fail-closed script. The universal macOS artifact is advertised under both darwin-x86_64 and darwin-aarch64. A missing file, empty signature or duplicate platform stops publication.
- Stage a GitHub release as a draft until artifacts finish uploading. This avoids exposing an empty release to the updater.
- Retain current-user Windows installation (no new administrator requirement), add English/Simplified Chinese/Traditional Chinese selection and the existing application icon. Arrange the macOS DMG application and Applications folder as a clear drag-to-install pair.

## Design references

Read actual upstream source rather than copying the visual style alone:

- Rayburst, commit 0c880e53b80c34321f540b76996d67035108bd32: src-tauri/tauri.conf.json. Adopted explicit NSIS language choice and clear DMG placement. Did not copy both-user installation, background services, browser flags or file associations unrelated to font management.
  https://github.com/AnInsomniacy/rayburst
- FlClash, commit be84736605527034689c95bf27828ff27d912c5e: windows/packaging/exe/inno_setup.iss. Its explicit update process handling reinforces lifecycle testing. Did not copy process-killing/service/proxy cleanup into a font manager.
  https://github.com/chen08209/FlClash
- Tauri updater schema requires OS-ARCH keys, not darwin-universal:
  https://v2.tauri.app/plugin/updater/
- NSIS options:
  https://v2.tauri.app/distribute/windows-installer/

## Verification and release boundary

Local release-manifest tests: 4 passed. cargo metadata --locked --no-deps passed. Cargo.lock update completed using Cargo 1.97.1. Full Rust/UI checks and installed-app upgrade testing must be recorded separately; a generated manifest is not an end-to-end updater test.

This branch does not bump the release version or publish a build. Shipped alpha.5 binaries remain unchanged until a tested replacement is explicitly released. Existing zbus/zvariant maintenance advisories are a separate migration task, not bypassed here.

Before release, test fresh installation, same-user upgrade, installation language, existing font retention, rollback, uninstall, and both macOS CPU architectures. Platform signing/notarization and real updater signature verification remain mandatory release checks.

Local Rust test attempt: `cargo test -p fontferry-core --locked` stopped at dependency build scripts because this machine has no MSVC `link.exe`. This is an environment blocker, not a passing test or a demonstrated code failure. Cross-platform CI is required before merge. Rustup downloaded the pinned toolchain but could not create its proxy hardlinks in this sandbox; Cargo was invoked directly from the installed toolchain directory.

## CI-discovered dependency repair (2026-11-15)

The independent RustSec job on PR #34 found 17 vulnerability records beyond the original TLS issue. The lockfile now updates quinn-proto to 0.11.19, quinn-udp to 0.5.16, tokio to 1.53.3, rustix to 1.1.6 and the yanked async-executor/chacha20 releases to 1.14.1/0.10.2. Bundled SQLite moves through rusqlite 0.41.1 to libsqlite3-sys 0.39.1. Direct ZIP and the updater ZIP dependency both move to zip 9.5.1; updater 2.14.1 requires Tauri 2.12, so this repair necessarily updates the Tauri runtime/build dependency family as well. No application identity, update endpoint, public key or version changes accompany it.

Our code uses the updater builder and check() without an allowDowngrades parameter or custom comparator. Signed update installation still requires disposable-profile acceptance testing. Existing font archive tests remain important because ZIP is a major-version migration; the font extractor continues to reject symlinks, path escapes and extraction-budget excess rather than calling ZipArchive::extract.

notify-rust 4.19.0 uses zbus 6, while the pinned notification plugin remains 2.4.0 to avoid introducing a second direct zbus 5 dependency. The opener and single-instance plugins still retain zbus 5 and its three unmaintained companion advisories. glib 0.18.5 also has a pre-existing unsound iterator advisory. These are not silently ignored or claimed resolved. Cross-platform CI, not local metadata/format checks, must validate the dependency migration before merge. No installer or font mutation was executed.

### Verified CI result

Commit a5354e2a254cda4880268b552e71eb5f962b5d2d was tested by GitHub Actions run 43675784376: Frontend, RustSec audit, and Rust Clippy/tests on Windows 2025, macOS 15 and Ubuntu 24.04 all passed. Linux logged 14 core tests and 31 platform tests, including all six recovery-opening cases. The remaining Dependency policy failure is exactly RUSTSEC-2026-0370/0371/0372 (zvariant, zbus_names, zvariant_derive maintenance notices retained through opener/single-instance). No new ignore entries were added. The draft PR must not be treated as merge-ready while this gate is red. Existing glib warning and real installer/update acceptance remain documented limitations, not successful test claims.

## Time-bounded maintenance-only exception

Decision window: 2026-11-15 00:00 UTC through (but not including) 2026-12-15 00:00 UTC. Only RUSTSEC-2026-0370, 0371 and 0372 are eligible. These describe standalone crates merged into zbus 6, not vulnerabilities. Existing official opener 2.5.5 and single-instance 2.4.4 retain zbus 5.18.0; notification already migrated through notify-rust 4.19.0. Replacing maintained Tauri integration with a private IPC fork is not justified solely to eliminate these notices.

The pinned cargo-deny 0.20.2 does not implement expiry/allow fields. Therefore scripts/maintenance-exceptions.mjs is a mandatory precondition of the Dependency policy job. It checks the absolute deadline, exact affected versions and their dependency consumers against cargo metadata, verifies each current official RustSec notice is still unmaintained, and fails on missing/network-invalid/changed notices. It rejects new cargo-audit ignores for these IDs. Each run emits warning annotations and a job summary. A daily workflow runs the dependency and audit jobs even without new commits. The three dated cargo-deny reasons only downgrade these exact IDs; other maintenance notices and vulnerabilities are not exempted. A change in dependency scope requires review, not silent widening.

At expiry the policy job fails. No automatic renewal is allowed. Maintainers must either migrate the official plugins and remove all three entries plus this validator, or review an explicit new decision. Once official plugins support zbus 6: update only the affected plugin chain, confirm zbus 5/zvariant/zbus_names/zvariant_derive disappear, remove the exceptions, run all platform tests plus Linux single-instance/window activation/external-link checks, and review updater and installer acceptance separately. Removing unused opener alone cannot remove the single-instance dependency.

The independent RustSec job and .cargo/audit.toml remain unchanged. A newly reported vulnerability on any affected package still blocks audit. The pre-existing glib unsound warning is not part of this exception and is not relabeled harmless. Code-merge readiness does not grant release approval or certify real font operations. Real upgrade, signature verification, app uninstall and retained management-data acceptance still require disposable-profile tests.

Local validation: all 14 script tests pass, including expiry boundary, changed versions/new consumers, removed dependencies, weakened scope, independent-audit suppression, and non-maintenance/malformed advisories. A real cargo metadata graph plus live official notices passed and emitted three explicit warnings. Independent review and full CI are required before merge.
