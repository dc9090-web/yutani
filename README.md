# Yutani

Yutani is a COSMIC-desktop companion for EVE Online multiboxing. It shows
live thumbnails of every EVE client as a layer-shell overlay, gives them
global hotkeys (`Ctrl+Alt+1..9` to focus the n-th client, `Ctrl+Alt+Right` /
`Ctrl+Alt+Left` to step through them), puts a panel applet and an
Applications launcher on the desktop, routes EVE's traffic — and only EVE's
— through an optional WireGuard tunnel, and wraps all of it in a settings
window (layouts, characters, tunnel, Steam).

## Requirements

- COSMIC 1.8 or newer on a Wayland session. `yutani doctor` prints every
  Wayland protocol it needs and whether cosmic-comp advertises it.
- Arch/CachyOS packages: `nftables`, `wireguard-tools`, `curl`, `polkit`,
  `gtk-update-icon-cache`, `rustup`, `git`, `base-devel`.
- A stable Rust toolchain (`rustup default stable`).

## Install

```bash
# 1. build
cargo build --release --locked

# 2. install both binaries root-owned — the tunnel unit refuses a
#    user-writable binary, and the panel starts the applet from the
#    directory `yutani` itself lives in
sudo install -o root -g root -m 0755 \
  target/release/yutani target/release/yutani-applet /usr/local/bin/

# 3. icons, the applet .desktop file and the Applications launcher
/usr/local/bin/yutani applet install

# 4. a systemd user unit that brings the daemon back after a crash
/usr/local/bin/yutani service install

# 5. the COSMIC keyboard shortcuts
/usr/local/bin/yutani shortcuts install
```

One manual step is left after step 3: **Settings → Desktop → Panel →
Applets → add "Yutani"**.

Then, for each EVE account in Steam, set the game's **Launch Options** to:

```
PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 yutani launch -- %command%
```

`PROTON_ENABLE_WAYLAND=1` gives each client a native Wayland toplevel that
Yutani can capture, `WINE_NO_WM_DECORATION=1` stops Wine drawing its own
title bar, and `yutani launch` starts the game inside the tunnel's cgroup so
the tunnel can pick it out. Spell the path out
(`/usr/local/bin/yutani launch -- %command%`) if Steam cannot find `yutani`
on `PATH`.

Every subcommand above is idempotent — re-run them after an upgrade.

## The EVE-only tunnel

Yutani can send EVE's traffic through a WireGuard exit while everything else
on the machine goes direct. It takes a wg-quick `.conf` (Proton VPN's
download works as-is):

- **Settings window → Tunnel**: browse for the file, type its path, or drop
  it on the window, then press **Install tunnel**.
- **Terminal**: `yutani tunnel install ~/Downloads/EVE-UK-455.conf`

Either way polkit asks for your password once, and the installer writes
`/etc/yutani/tunnel.conf` (mode 0600, root-only), a systemd unit and a
polkit rule that lets you start and stop the tunnel without a password
afterwards. `yutani tunnel connect` / `disconnect` / `status` and the
Tunnel page's buttons drive it from then on; `yutani tunnel uninstall`
removes all three files. Private keys are never printed or logged.

## Deploying with Ansible

`deploy/ansible/` has a playbook and a `yutani` role that does everything on
this page — packages, build, root-owned binaries, applet, service,
shortcuts and the optional tunnel — on a fresh Arch/CachyOS COSMIC machine,
idempotently:

```bash
cd deploy/ansible
ansible-playbook -K playbook.yml -e yutani_user=<your user>
```

See [deploy/ansible/README.md](deploy/ansible/README.md) for the variables
and for installing the tunnel in the same run.

## Everyday commands

| Command | What it does |
| --- | --- |
| `yutani start` | Start the daemon (through the user unit when installed). |
| `yutani show` / `hide` / `toggle` | Show or hide the thumbnails. |
| `yutani focus N`, `next`, `prev` | Focus a client in layout order. |
| `yutani layouts`, `yutani layout <name>` | List and apply saved layouts. |
| `yutani settings` | Open the settings window. |
| `yutani status` | The daemon's state (clients, visibility, tunnel) as JSON. |
| `yutani doctor` | Check the compositor for everything Yutani needs. |
| `yutani quit` | Ask the running daemon to exit. |

Configuration lives in `~/.config/yutani/config.ron`; the settings window
writes the same file.
