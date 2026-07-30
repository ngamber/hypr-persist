# AGENTS.md

Guidance for agents working in this repo.

## What this is

`hypr-persist` is a session persistence daemon for Hyprland: it saves open applications
(with workspace, floating geometry, and dwindle/master tiling layout) and restores them
after logout/reboot. Binary crate only — no `lib.rs`; everything is private to `main.rs`'s
`mod` declarations (`config`, `core`, `ipc`, `models`, `resolver`, `tests`).

Originally forked from `IraSkyx/hyprresume`, substantially reworked. Rust 2024 edition,
toolchain pinned to 1.94 (`rust-toolchain.toml`).

## Build, lint, test

Always go through `make`, not raw `cargo`, to match CI exactly:

```sh
make build   # cargo build
make format  # cargo fmt --all (pass FMT_FLAGS="-- --check" to check without writing)
make lint    # cargo clippy --all-targets, deny suspicious/style/nursery/pedantic/all + warnings
make test    # cargo nextest run --all (requires cargo-nextest installed)
```

`make lint` denies `clippy::nursery` and `clippy::pedantic` in addition to the defaults —
expect it to flag things plain `cargo clippy` wouldn't (doc-comment backticks around
identifiers, `into_iter()` on single-item array literals instead of `std::iter::once`,
missing `const fn`, `too_many_lines`, redundant `pub(crate)` inside an already-private
module, etc). Run `make lint` locally before pushing; CI (`.github/workflows/ci.yml`,
pull_request-triggered only) runs build → format-check → lint → test in that order and
stops at the first failure.

Several long orchestration functions in `src/core/restore.rs` (`restore_master`,
`launch_and_track`) already carry `#[allow(clippy::too_many_arguments, clippy::too_many_lines)]`
— they're intentionally complex state-machine functions. Prefer extending that existing
allow-list over fighting the lint if a change legitimately grows one of them further; don't
add blanket allows elsewhere.

## Source layout

- `src/core/restore.rs` — `RestoreEngine`: the restore state machine (dwindle + master
  paths, live-window adoption, splash/racing-window handling, BSP placement).
- `src/core/state.rs` — `StateManager`: tracks live windows, include/exclude rules.
- `src/core/layout/{dwindle,master}.rs` — pure geometry/tree inference from saved window
  positions, no I/O.
- `src/core/daemon.rs`, `src/core/snapshot.rs` — daemon loop and session save/load.
- `src/ipc/lua_compat.rs` — translates classic `hyprctl dispatch <string>` syntax into
  `hl.dsp.*` Lua calls. Needed because Lua-configured Hyprland rejects the classic dispatch
  string syntax outright. Don't assume the classic syntax works when testing dispatches.
- `src/resolver/` — maps a window class back to a launch command via `.desktop` files,
  Flatpak cgroups, or `/proc`.
- `src/tests/{cli,mock_ipc,simulation}.rs` — integration-style tests; most unit tests are
  co-located in `#[cfg(test)] mod tests` blocks within their source file.

## Release process

Version bumps follow a strict four-step pattern (see history for `v0.1.0`/`v0.1.1`).
Do steps 1-3 as separate PRs/commits, in this order:

1. `chore: bump version to X.Y.Z` — update `Cargo.toml`, run `cargo build` to refresh
   `Cargo.lock`'s version entry, and update the hardcoded version string asserted in
   `src/tests/cli.rs`'s `cli_version` test.
2. `chore: update PKGBUILD for X.Y.Z` — bump `pkg/aur/PKGBUILD`'s `pkgver`, set
   `sha256sums=('SKIP')` with a `# TODO: replace once the vX.Y.Z tag is pushed...` comment
   (the release tarball doesn't exist yet).
3. After merging, tag `vX.Y.Z` (annotated, `git tag -a`) on `main` and push the tag. Then
   compute the real hash — `curl -sL "$url/archive/vX.Y.Z.tar.gz" | sha256sum` — and open
   `chore: fill in sha256sum for the vX.Y.Z release tarball`, replacing `SKIP`.
4. Publish to AUR — this repo's `pkg/aur/PKGBUILD` is the source of truth, but it isn't
   what `pacman`/AUR helpers install from. Clone `ssh://aur@aur.archlinux.org/hypr-persist.git`
   separately, copy in the finalized `PKGBUILD` from step 3, regenerate `.SRCINFO` with
   `makepkg --printsrcinfo > .SRCINFO`, and commit/push directly to `master` (AUR repos have
   no PR mechanism — direct push is the only way to publish). Verify with a `makepkg -f`
   build first.

## Live testing caveats

Hyprland dispatch behavior can only be meaningfully verified against a real running
Hyprland session, not just unit tests. When doing so: always target an explicit
`address:0x..` selector rather than relying on ambient focus (races against whatever the
user is actually doing), and don't assume a manual live fix persists to the next save —
session state is periodic/SIGTERM-triggered, so an unrelated event can overwrite it first.
