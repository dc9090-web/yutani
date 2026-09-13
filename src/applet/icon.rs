//! Which panel icon the tunnel's state calls for (spec §3).

use crate::tunnel::status::TunnelStatus;

/// A tunnel that is up but has not handshaked for this long is in trouble.
pub const HANDSHAKE_STALE_S: u64 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    /// The plain mark: connected and healthy, or deliberately disconnected.
    Plain,
    /// The blue mark: the tunnel is connected *and* there is an EVE client
    /// behind it. It is the one state where Yutani is doing its whole job,
    /// and the only one that keeps a colour of its own on the panel.
    Active,
    /// Ring badge: an action is in flight, or the tunnel has just come up
    /// and is still handshaking.
    Sync,
    /// Exclamation badge: up with no handshake for `HANDSHAKE_STALE_S`
    /// (including one that never handshaked at all), or a failed unit.
    Attention,
    /// The plain mark at 38 %: no daemon, or no tunnel installed.
    Dim,
}

impl IconState {
    /// The icon-theme name `yutani applet install` writes (the applet draws
    /// the same bytes from `crate::assets`).
    pub fn icon_name(self) -> &'static str {
        match self {
            IconState::Sync => "y-sync-symbolic",
            IconState::Attention => "y-attention-symbolic",
            IconState::Active => "y-color",
            IconState::Plain | IconState::Dim => "y-symbolic",
        }
    }

    pub fn opacity(self) -> f32 {
        if self == IconState::Dim { super::theme::DIM_OPACITY } else { 1.0 }
    }

    /// Whether the panel may recolour this icon to its own ink. Every state
    /// but [`IconState::Active`] is a single-colour mark that *should*
    /// follow the panel; `Active` is the full-colour `y-color` mark, and
    /// drawing it symbolic would tint its blue away — which is the entire
    /// point of the state.
    pub fn symbolic(self) -> bool {
        self != IconState::Active
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            IconState::Sync => crate::assets::Y_SYNC_SYMBOLIC,
            IconState::Attention => crate::assets::Y_ATTENTION_SYMBOLIC,
            IconState::Active => crate::assets::Y_COLOR,
            IconState::Plain | IconState::Dim => crate::assets::Y_SYMBOLIC,
        }
    }
}

/// `tunnel` is `None` when the daemon did not answer. `clients` is how many
/// EVE clients the daemon is tracking — it is what separates the blue
/// [`IconState::Active`] mark from the plain one. `pending` is true for
/// [`super::PENDING_S`] after a `tunnel connect|disconnect` was sent.
pub fn icon_state(tunnel: Option<&TunnelStatus>, clients: usize, pending: bool) -> IconState {
    let Some(t) = tunnel else { return IconState::Dim };
    if !t.installed {
        return IconState::Dim;
    }
    if pending {
        return IconState::Sync;
    }
    // Spec §3 "…or unit failed": systemd's verdict outranks whatever the
    // interface and the (possibly stale) status file say.
    if t.failed {
        return IconState::Attention;
    }
    match (t.connected, t.handshake_age_s) {
        (false, _) => IconState::Plain,
        // Up and never handshaked. That is *settling* only for as long as
        // the link has just come up; a tunnel still silent after
        // HANDSHAKE_STALE_S is not handshaking, it is broken, and the sync
        // badge must stop spinning forever. A daemon too old to send
        // `up_for_s` gives nothing to time out on, so it keeps the old
        // behaviour rather than raising a false alarm.
        (true, None) => {
            if t.up_for_s.is_none_or(|up| up < HANDSHAKE_STALE_S) {
                IconState::Sync
            } else {
                IconState::Attention
            }
        }
        (true, Some(age)) if age >= HANDSHAKE_STALE_S => IconState::Attention,
        // A healthy tunnel with EVE behind it: the one state worth a
        // colour. With nothing running the tunnel is merely ready, which is
        // the plain mark — the panel must not claim more than is true.
        (true, Some(_)) if clients > 0 => IconState::Active,
        (true, Some(_)) => IconState::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::status::TunnelStatus;

    fn tunnel(installed: bool, connected: bool, handshake_age_s: Option<u64>) -> TunnelStatus {
        TunnelStatus {
            installed,
            connected,
            iface: "yutani0".into(),
            location: "London".into(),
            address: connected.then(|| "10.2.0.2".to_string()),
            endpoint: connected.then(|| "198.51.100.10:51820".to_string()),
            handshake_age_s,
            // Freshly up unless a test says otherwise.
            up_for_s: connected.then_some(1),
            failed: false,
            exit_address: None,
            rx_bytes: 0,
            tx_bytes: 0,
        }
    }

    /// A link that is up but has never handshaked: settling at first, but a
    /// tunnel still silent after [`HANDSHAKE_STALE_S`] is in exactly the
    /// trouble a stale handshake is, and must stop spinning forever.
    fn never_handshaked(up_for_s: Option<u64>) -> TunnelStatus {
        TunnelStatus { up_for_s, ..tunnel(true, true, None) }
    }

    #[test]
    fn no_daemon_and_no_tunnel_are_both_dim() {
        assert_eq!(icon_state(None, 0, false), IconState::Dim);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), 0, false), IconState::Dim);
        // Even a pending action cannot brighten a missing daemon.
        assert_eq!(icon_state(None, 0, true), IconState::Dim);
    }

    #[test]
    fn a_pending_action_shows_the_sync_icon() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 0, true), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), 0, true), IconState::Sync);
    }

    #[test]
    fn a_deliberate_disconnect_and_a_fresh_handshake_are_both_plain() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 0, false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(0))), 0, false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(179))), 0, false), IconState::Plain);
    }

    #[test]
    fn up_without_a_handshake_syncs_and_a_stale_handshake_demands_attention() {
        assert_eq!(icon_state(Some(&tunnel(true, true, None)), 0, false), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(180))), 0, false), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(6_000))), 0, false), IconState::Attention);
    }

    /// The sync badge is for a tunnel that is *settling*, so it has to be
    /// bounded: a link that has been up for [`HANDSHAKE_STALE_S`] without a
    /// single handshake is not handshaking, it is broken.
    #[test]
    fn a_link_that_never_handshakes_stops_syncing_and_demands_attention() {
        assert_eq!(icon_state(Some(&never_handshaked(Some(0))), 0, false), IconState::Sync);
        assert_eq!(icon_state(Some(&never_handshaked(Some(179))), 0, false), IconState::Sync);
        assert_eq!(icon_state(Some(&never_handshaked(Some(180))), 0, false), IconState::Attention);
        assert_eq!(icon_state(Some(&never_handshaked(Some(9_000))), 0, false), IconState::Attention);
        // A pending connect still owns the icon for its own 10 s.
        assert_eq!(icon_state(Some(&never_handshaked(Some(9_000))), 0, true), IconState::Sync);
        // An older daemon sends no uptime at all; with nothing to time out
        // on, it keeps the pre-`up_for_s` behaviour rather than crying wolf.
        assert_eq!(icon_state(Some(&never_handshaked(None)), 0, false), IconState::Sync);
    }

    /// Spec §3: "or unit failed". systemd calling the unit failed outranks
    /// everything the interface itself says — including a link that looks
    /// perfectly healthy from a stale status file.
    #[test]
    fn a_failed_unit_demands_attention_whatever_the_link_looks_like() {
        let failed = |t: TunnelStatus| TunnelStatus { failed: true, ..t };
        assert_eq!(icon_state(Some(&failed(tunnel(true, false, None))), 0, false), IconState::Attention);
        assert_eq!(icon_state(Some(&failed(tunnel(true, true, Some(0)))), 0, false), IconState::Attention);
        // Nothing is installed: still nothing to show but the dim mark.
        assert_eq!(icon_state(Some(&failed(tunnel(false, false, None))), 0, false), IconState::Dim);
        // A press that is still settling keeps the sync badge — the user
        // just asked for this, and the unit may be on its way back up.
        assert_eq!(icon_state(Some(&failed(tunnel(true, false, None))), 0, true), IconState::Sync);
    }

    /// The whole point of the blue mark: the tunnel is up, healthy, *and*
    /// carrying an EVE client. Anything short of all three is not it.
    #[test]
    fn the_blue_mark_needs_a_healthy_tunnel_and_a_client_behind_it() {
        let healthy = tunnel(true, true, Some(21));
        assert_eq!(icon_state(Some(&healthy), 1, false), IconState::Active);
        assert_eq!(icon_state(Some(&healthy), 9, false), IconState::Active);
        // Connected but nothing is running: ready, not active.
        assert_eq!(icon_state(Some(&healthy), 0, false), IconState::Plain);
        // Clients but no tunnel: the icon is about the tunnel first.
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 3, false), IconState::Plain);
        // A stale handshake is trouble however many clients are running.
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(180))), 3, false), IconState::Attention);
        assert_eq!(icon_state(Some(&never_handshaked(Some(9_000))), 3, false), IconState::Attention);
        // Still settling, or a press still in flight: sync owns the icon.
        assert_eq!(icon_state(Some(&never_handshaked(Some(1))), 3, false), IconState::Sync);
        assert_eq!(icon_state(Some(&healthy), 3, true), IconState::Sync);
        // A failed unit outranks it, and a missing daemon or tunnel is dim.
        assert_eq!(icon_state(Some(&TunnelStatus { failed: true, ..healthy.clone() }), 3, false), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), 3, false), IconState::Dim);
        assert_eq!(icon_state(None, 3, false), IconState::Dim);
    }

    /// `y-color` is the only icon the panel must not recolour: it is
    /// installed to `scalable/apps` precisely because a symbolic tint would
    /// throw its blue away.
    #[test]
    fn only_the_blue_mark_refuses_the_panel_tint() {
        assert!(!IconState::Active.symbolic());
        for state in [IconState::Plain, IconState::Sync, IconState::Attention, IconState::Dim] {
            assert!(state.symbolic(), "{state:?}");
        }
        assert_eq!(IconState::Active.bytes(), crate::assets::Y_COLOR);
        assert_eq!(IconState::Active.opacity(), 1.0);
    }

    #[test]
    fn each_state_names_one_of_the_installed_icons() {
        assert_eq!(IconState::Plain.icon_name(), "y-symbolic");
        assert_eq!(IconState::Sync.icon_name(), "y-sync-symbolic");
        assert_eq!(IconState::Attention.icon_name(), "y-attention-symbolic");
        assert_eq!(IconState::Dim.icon_name(), "y-symbolic");
        assert_eq!(IconState::Active.icon_name(), "y-color");
        // Every name is one of the files `applet install` actually writes.
        for state in [IconState::Plain, IconState::Sync, IconState::Attention, IconState::Dim, IconState::Active] {
            let file = format!("{}.svg", state.icon_name());
            assert!(crate::assets::ICONS.iter().any(|(_, n, _)| *n == file), "{file}");
        }
        assert_eq!(IconState::Plain.opacity(), 1.0);
        assert_eq!(IconState::Dim.opacity(), 0.38);
    }
}
