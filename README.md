# hypr-persist

Session persistence for [Hyprland](https://hyprland.org). Saves your open applications and puts them back on the right workspaces after a reboot.

- Resolves launch commands from `.desktop` files, Flatpak cgroups or `/proc`
- Saves sessions as human-readable TOML
- Restores apps to their original workspaces with floating window geometry
- Reconstructs tiling layouts from saved window positions (dwindle and master layouts)

Originally forked from [hyprresume](https://github.com/IraSkyx/hyprresume) by Adrien Lenoir;
substantially reworked since (see `LICENSE` for attribution).

## Install

**From source:**

```sh
cargo install --path .
```

## Usage

Add to your `hyprland.conf`:

```conf
exec-once = hypr-persist
```

This starts the daemon, which restores your previous session, tracks window changes and saves periodically. It also saves on logout/shutdown via SIGTERM.

### Commands

```sh
hypr-persist save [name]      # snapshot current session
hypr-persist restore [name]   # restore a saved session
hypr-persist list             # list saved sessions
hypr-persist delete <name>    # delete a session
hypr-persist resolve <class>  # show what command a window class maps to
hypr-persist status           # show daemon/session info
```

Use `-v` for info logs, `-vv` for debug, `-vvv` for trace.

## Configuration

Place a config file at `~/.config/hypr/hypr-persist.toml`. All fields are optional; defaults are sane.

```toml
[general]
save_interval = 120           # seconds between auto-saves
session_dir = "~/.local/share/hypr-persist"
restore_on_start = true
per_window_launch = true      # one process per window, not per unique app
restore_geometry = true       # restore floating window position/size
restore_layout = true         # reconstruct tiling layout (dwindle + master)

[rules]
exclude = [
    "^xdg-desktop-portal.*",
    "^org\\.kde\\.polkit.*",
]
# include = ["^firefox$", "^code$"]  # allowlist mode

[overrides]
# "app.zen_browser.zen" = "flatpak run app.zen_browser.zen"
# "steam_app_.*" = ""  # empty = skip
```

A full example config is in [`assets/hypr-persist.toml`](assets/hypr-persist.toml).

### Tiling layout restoration

When `restore_layout` is enabled (the default), hypr-persist saves every tiled window's position and size and reconstructs the layout on restore. The active layout is auto-detected via `general:layout`.

**Dwindle** — infers a BSP tree from saved geometry, replays it via `layoutmsg preselect`, and corrects split ratios with `layoutmsg splitratio` where reachable and targeted pixel resizes for splits it can't reach directly. Your Hyprland config should include `preserve_split = true` so that split directions are stable:

```conf
dwindle {
    preserve_split = true
}
```

**Master** — infers orientation, master/stack partition and `mfact` from saved geometry. Windows are opened in the correct order (masters first, then stack) so the layout engine places them naturally.

Unsupported layouts fall back to simple restore: windows land on the correct workspaces but use Hyprland's default placement.

## Contributing

Requires Rust 1.94 (pinned in `rust-toolchain.toml`).

```sh
git clone https://github.com/ngamber/hypr-persist.git
cd hypr-persist
```

Build and test:

```sh
make build      # dev build
make test       # run all tests
make lint       # clippy (pedantic)
make format     # rustfmt
```

The test suite runs entirely without a live Hyprland instance. It uses mock IPC sockets and simulated sessions.

## License

BSD-3-Clause
