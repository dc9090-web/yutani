# Yutani — Tunnel settings page and Ansible deployment

**Status:** requested by Daniel 2026-09-13 ("make this deployable via an
Ansible playbook" and "add a place in settings where you can drop the Proton
VPN config file"). Plan: `docs/superpowers/plans/2026-09-13-yutani-tunnel-page-and-ansible.md`.

## 1. Tunnel page in the settings window

A fifth editable page, **Tunnel**, between Characters and Steam. It replaces
the terminal step `yutani tunnel install <file>` with a window that takes the
WireGuard configuration file (Proton VPN's wg-quick `.conf` download) three
ways: a **Browse…** button (the XDG file-chooser portal, filtered to
`*.conf`), a path typed into a text field, or **dropping the file onto the
settings window**. One **Install tunnel** button then runs the existing
`tunnel::install::install` on the blocking pool; that path shells out to
`pkexec`, so the desktop's polkit agent asks for the password once, exactly
as the CLI does. Replacing an installed tunnel with a new file is the same
button ("Replace configuration" while one is installed).

The page also shows the tunnel's state (not installed / installed as
`<location>` / connected / disconnected / unit failed) with **Connect**,
**Disconnect** and **Uninstall** buttons that run the same functions the IPC
`tunnel connect|disconnect` and `yutani tunnel uninstall` commands run. The
DNS settings (`tunnel.dns_servers`, `tunnel.dns_domains` from `config.ron`)
are shown read-only with the note that the tunnel must be re-installed after
changing them (they are baked into `/etc/yutani/tunnel.conf` at install).

Safety and feedback follow the rest of the window: the note line reports
what happened (`tunnel installed from EVE-UK-455.conf; press Connect`,
`install cancelled or failed: …`), the buttons are disabled while an action
is in flight, and nothing is written when the path does not name a readable
file. Private keys are never displayed or logged; the page shows only the
file name and the parsed `location`.

Every state change on this page is a file operation outside `config.ron`,
so the page works while `config.ron` is broken (like Layouts, Characters and
Steam).

## 2. Ansible deployment

`deploy/ansible/` holds a playbook and one role, `yutani`, that takes a
fresh COSMIC machine (Arch/CachyOS today; other package managers stop with a
clear message) to a fully installed Yutani:

1. Packages (root): `nftables`, `wireguard-tools`, `curl`, `polkit`,
   `gtk-update-icon-cache`, `rustup`, `git`, `base-devel`. A stable Rust
   toolchain is selected for the user when none is.
2. Source (user): clone `yutani_repo` at `yutani_version` into
   `yutani_src_dir`, `cargo build --release --locked`.
3. Binaries (root): `yutani` and `yutani-applet` into `yutani_prefix`
   (`/usr/local/bin`), owner root, mode 0755 — the tunnel unit refuses a
   user-writable binary, so this ordering is not optional.
4. User integration (user, with the user's `XDG_RUNTIME_DIR` and session
   bus): `yutani applet install`, `yutani service install` (and
   `systemctl --user enable yutani.service` when
   `yutani_enable_service_at_login`), `yutani shortcuts install` when
   `yutani_install_shortcuts`.
5. Tunnel (root, only when `yutani_wg_conf` names a file on the controller):
   the file is copied to the target with mode 0600, `yutani tunnel
   install-root --conf … --uid … --user … --exe <prefix>/yutani
   [--dns-servers …] [--dns-domains …]` is run as root — the non-interactive
   twin of the pkexec path — and the copy is removed. The playbook never
   prints the file.

The playbook is idempotent: every Yutani subcommand it runs is (they rewrite
the same files), and the build step reports `changed` only when cargo
compiled something. A top-level `README.md` documents the manual install and
the Ansible one.

## 3. Out of scope

Editing the WireGuard file in the window; multiple tunnel profiles; Debian
or Fedora package lists (the role's package map has one entry, `pacman`, and
a place for more); building the applet's `.desktop` files into a distro
package.
