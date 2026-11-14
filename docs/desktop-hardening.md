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
