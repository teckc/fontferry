# Linux packaging and operation-lock repair

This patch reconstructs the previously reviewed Linux packaging/lock work on main
`ce5e47c`. The former private checkout and commit `99047585` were unavailable in
the current execution environment and that commit was never published remotely.
This is a new implementation/commit, not a claim to reproduce the old Git object.
It excludes later XDG path, scheduler and observed-font experiments.

## Changes

- Align the locked JavaScript Tauri API (2.12.4) with the Rust library's 2.12
  major/minor (2.12.5), leaving CLI and other dependencies unchanged. A script
  regression checks locked major/minor compatibility.
- Explicitly declare `fontconfig` for DEB and RPM packaging. GTK's fontconfig
  library dependency is not the command package containing `fc-cache`.
- Check `fc-cache --version` before starting Linux file mutations/backups/journal
  creation and before committed cleanup. This reduces missing-command failures
  after mutation; it does not promise that later `fc-cache -f` cannot fail.
- Replace the raw operation lock file with a non-cloneable RAII guard. It explicitly
  unlocks in the acquiring process before closing. Unix fork inherits the same
  open-file description, so closing the parent's last descriptor alone can leave
  the lock held until a transient child execs/exits. A PID guard prevents a forked
  child dropping its copy from unlocking its parent's active operation.

## Validation boundary

The reconstruction runs isolated core/platform tests and clippy, script tests,
frontend type/component/build checks, plus real child-process lock regressions and
an optional Python fork reproducer. Native missing-command detection runs with an
empty PATH in a subprocess; lifecycle mutation boundaries use temporary fonts and
a fake font-system implementation. No user fonts, services or settings are changed.

The earlier debug DEB acceptance belongs to the old inaccessible commit and is not
claimed for this reconstructed tree. Fresh cross-platform full-workspace tests and
policy checks must pass on the new PR HEAD. This patch is not a new signed release,
real GUI acceptance, RPM installation test or hardware power-failure test.
