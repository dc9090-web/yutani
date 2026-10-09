# Deploying Yutani with Ansible

One role, `yutani`, takes a fresh COSMIC machine (Arch/CachyOS today) to a
fully installed Yutani. Re-running it is safe: every Yutani subcommand it
calls rewrites the same files, and only two things ever report `changed`
— the build when cargo actually compiled something, and the binaries when
they differ from the installed ones.

## What it does

1. **Packages** (root): `curl`, `gtk-update-icon-cache`, `rustup`, `git`, `base-devel`. A stable Rust
   toolchain is selected for the user when none is. Skipped when
   `yutani_manage_packages: false`; a non-pacman host stops with a clear
   message instead of guessing package names.
2. **Source** (user): clone `yutani_repo` at `yutani_version` into
   `yutani_src_dir` and `cargo build --release --locked`.
3. **Binaries** (root): `yutani` and `yutani-applet` into `yutani_prefix`,
   owner `root`, mode `0755`.
4. **User integration** (as the desktop user, with their `XDG_RUNTIME_DIR`
   and session bus): `yutani applet install`, `yutani service install`
   (plus `systemctl --user enable yutani.service` when
   `yutani_enable_service_at_login`), a restart of the daemon when new
   binaries were installed and it is running under its unit (a daemon that
   is not running is left alone), and `yutani shortcuts install`.
## Variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `yutani_user` | `""` | **Required.** The desktop user Yutani runs as. |
| `yutani_repo` | GitHub URL | Source repository to clone. |
| `yutani_version` | `master` | Branch, tag or commit to build. |
| `yutani_src_dir` | `/home/<user>/.local/src/yutani` | Where the clone lives. |
| `yutani_prefix` | `/usr/local/bin` | Where both binaries are installed. |
| `yutani_manage_packages` | `true` | Install the package list. |
| `yutani_install_shortcuts` | `true` | Write the COSMIC keyboard shortcuts. |
| `yutani_install_service` | `true` | Write the systemd user unit. |
| `yutani_enable_service_at_login` | `false` | Also enable the unit. |
| `yutani_packages` | pacman list | Package map, keyed by `ansible_facts.pkg_mgr`. |

## Running it

Copy `inventory.example.ini`, set `yutani_user`, then from this directory:

```bash
ansible-playbook -K playbook.yml
```

`-K` asks for the become password once. For a remote host, replace the
`localhost ansible_connection=local` line with the host's name.

Dry run, to see what would change without touching the machine:

```bash
ansible-playbook -K --check --diff playbook.yml -e yutani_user=<user>
```

Check mode skips every `command` task (build, applet, service, shortcuts),
so what it reports on is the packages, the clone and the binaries.
It still needs `-K` for the root steps, and on a machine that has never been
deployed it stops at the binary copy: each step after the clone consumes
files an earlier step would have produced, and check mode produced none of
them.

## After it runs

One manual step is left, as with the hand install: **Settings → Desktop →
Panel → Applets → add "Yutani"**.

A running daemon picks up new binaries on the spot (the play restarts its
unit). The panel applet does not: cosmic-panel keeps the process it
started, so after an upgrade the applet runs the old binary until the panel
is restarted — log out and in, or `pkill -x cosmic-panel` (the session
respawns it).
