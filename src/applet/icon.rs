//! Which panel icon the tunnel's state calls for (redesign spec §1).
//!
//! The glyph is never recoloured: every state is the panel's own ink, at
//! full strength or at 40 %, and the news is a dot in the bottom-right
//! corner — filled success for a working tunnel, filled warning for one in
//! trouble, a hollow ring while an action is settling.

use crate::tunnel::status::TunnelStatus;

/// A tunnel that is up but has not handshaked for this long is in trouble.
pub const HANDSHAKE_STALE_S: u64 = 180;

/// The panel icon: the solid mark below this many pixels, the two-piece
/// mark (with the slice through the stem) from here up.
pub const TWO_PIECE_MIN_PX: u16 = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    /// The plain mark: connected and healthy with nothing behind it, or
    /// deliberately disconnected.
    Plain,
    /// The mark with the success dot: the tunnel is connected *and* there
    /// is an EVE client behind it — the one state where Yutani is doing its
    /// whole job.
    Active,
    /// The mark with the hollow ring: an action is in flight, or the tunnel
    /// has just come up and is still handshaking.
    Sync,
    /// The mark with the warning dot: up with no handshake for
    /// `HANDSHAKE_STALE_S` (including one that never handshaked at all), or
    /// a failed unit.
    Attention,
    /// The mark at 40 %: no daemon, or no tunnel installed.
    Dim,
}

/// What is drawn over the mark's bottom-right corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// Filled, the theme's success colour.
    Connected,
    /// Filled, the theme's warning colour.
    Attention,
    /// Hollow: panel-background fill, ink ring.
    Sync,
}

impl IconState {
    /// The icon-theme name `yutani applet install` writes. One name for
    /// every state: the shape never changes, only its opacity and badge.
    pub fn icon_name(self) -> &'static str {
        crate::assets::SYMBOLIC_NAME
    }

    pub fn opacity(self) -> f32 {
        if self == IconState::Dim { super::theme::DIM_OPACITY } else { 1.0 }
    }

    pub fn badge(self) -> Option<Badge> {
        match self {
            IconState::Active => Some(Badge::Connected),
            IconState::Attention => Some(Badge::Attention),
            IconState::Sync => Some(Badge::Sync),
            IconState::Plain | IconState::Dim => None,
        }
    }

    /// The mark's bytes for a panel icon `icon_px` tall: the solid variant
    /// below [`TWO_PIECE_MIN_PX`], where the slice would land on half a
    /// pixel, the two-piece mark otherwise.
    pub fn bytes(self, icon_px: u16) -> &'static [u8] {
        if icon_px < TWO_PIECE_MIN_PX { crate::assets::YUTANI_SYMBOLIC_16 } else { crate::assets::YUTANI_SYMBOLIC }
    }
}

/// `tunnel` is `None` when the daemon did not answer. `clients` is how many
/// EVE clients the daemon is tracking — it is what separates the dotted
/// [`IconState::Active`] mark from the plain one. `pending` is true for
/// [`super::PENDING_S`] after a `tunnel connect|disconnect` was sent.
/// `steam_problem` is true when the daemon found a Steam account whose EVE
/// launch line is broken (`Status::steam`): the warning dot, unless the
/// mark is dim anyway.
pub fn icon_state(tunnel: Option<&TunnelStatus>, clients: usize, pending: bool, steam_problem: bool) -> IconState {
    let Some(t) = tunnel else { return IconState::Dim };
    if !t.installed {
        return IconState::Dim;
    }
    // A launch line that will fail (or bypass the tunnel) is set-up news
    // the user must see before pressing Play; it outranks a pending
    // action's sync ring.
    if steam_problem {
        return IconState::Attention;
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
        // A healthy tunnel with EVE behind it: the one state worth the
        // dot. With nothing running the tunnel is merely ready, which is
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
        assert_eq!(icon_state(None, 0, false, false), IconState::Dim);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), 0, false, false), IconState::Dim);
        // Even a pending action cannot brighten a missing daemon.
        assert_eq!(icon_state(None, 0, true, false), IconState::Dim);
    }

    #[test]
    fn a_pending_action_shows_the_sync_icon() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 0, true, false), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), 0, true, false), IconState::Sync);
    }

    #[test]
    fn a_deliberate_disconnect_and_a_fresh_handshake_are_both_plain() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 0, false, false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(0))), 0, false, false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(179))), 0, false, false), IconState::Plain);
    }

    #[test]
    fn up_without_a_handshake_syncs_and_a_stale_handshake_demands_attention() {
        assert_eq!(icon_state(Some(&tunnel(true, true, None)), 0, false, false), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(180))), 0, false, false), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(6_000))), 0, false, false), IconState::Attention);
    }

    /// The sync badge is for a tunnel that is *settling*, so it has to be
    /// bounded: a link that has been up for [`HANDSHAKE_STALE_S`] without a
    /// single handshake is not handshaking, it is broken.
    #[test]
    fn a_link_that_never_handshakes_stops_syncing_and_demands_attention() {
        assert_eq!(icon_state(Some(&never_handshaked(Some(0))), 0, false, false), IconState::Sync);
        assert_eq!(icon_state(Some(&never_handshaked(Some(179))), 0, false, false), IconState::Sync);
        assert_eq!(icon_state(Some(&never_handshaked(Some(180))), 0, false, false), IconState::Attention);
        assert_eq!(icon_state(Some(&never_handshaked(Some(9_000))), 0, false, false), IconState::Attention);
        // A pending connect still owns the icon for its own 10 s.
        assert_eq!(icon_state(Some(&never_handshaked(Some(9_000))), 0, true, false), IconState::Sync);
        // An older daemon sends no uptime at all; with nothing to time out
        // on, it keeps the pre-`up_for_s` behaviour rather than crying wolf.
        assert_eq!(icon_state(Some(&never_handshaked(None)), 0, false, false), IconState::Sync);
    }

    /// Spec §3: "or unit failed". systemd calling the unit failed outranks
    /// everything the interface itself says — including a link that looks
    /// perfectly healthy from a stale status file.
    #[test]
    fn a_failed_unit_demands_attention_whatever_the_link_looks_like() {
        let failed = |t: TunnelStatus| TunnelStatus { failed: true, ..t };
        assert_eq!(icon_state(Some(&failed(tunnel(true, false, None))), 0, false, false), IconState::Attention);
        assert_eq!(icon_state(Some(&failed(tunnel(true, true, Some(0)))), 0, false, false), IconState::Attention);
        // Nothing is installed: still nothing to show but the dim mark.
        assert_eq!(icon_state(Some(&failed(tunnel(false, false, None))), 0, false, false), IconState::Dim);
        // A press that is still settling keeps the sync badge — the user
        // just asked for this, and the unit may be on its way back up.
        assert_eq!(icon_state(Some(&failed(tunnel(true, false, None))), 0, true, false), IconState::Sync);
    }

    /// The whole point of the blue mark: the tunnel is up, healthy, *and*
    /// carrying an EVE client. Anything short of all three is not it.
    #[test]
    fn the_blue_mark_needs_a_healthy_tunnel_and_a_client_behind_it() {
        let healthy = tunnel(true, true, Some(21));
        assert_eq!(icon_state(Some(&healthy), 1, false, false), IconState::Active);
        assert_eq!(icon_state(Some(&healthy), 9, false, false), IconState::Active);
        // Connected but nothing is running: ready, not active.
        assert_eq!(icon_state(Some(&healthy), 0, false, false), IconState::Plain);
        // Clients but no tunnel: the icon is about the tunnel first.
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 3, false, false), IconState::Plain);
        // A stale handshake is trouble however many clients are running.
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(180))), 3, false, false), IconState::Attention);
        assert_eq!(icon_state(Some(&never_handshaked(Some(9_000))), 3, false, false), IconState::Attention);
        // Still settling, or a press still in flight: sync owns the icon.
        assert_eq!(icon_state(Some(&never_handshaked(Some(1))), 3, false, false), IconState::Sync);
        assert_eq!(icon_state(Some(&healthy), 3, true, false), IconState::Sync);
        // A failed unit outranks it, and a missing daemon or tunnel is dim.
        assert_eq!(icon_state(Some(&TunnelStatus { failed: true, ..healthy.clone() }), 3, false, false), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), 3, false, false), IconState::Dim);
        assert_eq!(icon_state(None, 3, false, false), IconState::Dim);
    }

    /// Redesign spec §1: the glyph is never recoloured. Status is a corner
    /// badge — filled success, filled warning, or a hollow ring — and the
    /// stopped state is the same shape at 40 %.
    #[test]
    fn status_is_a_corner_badge_and_never_a_recoloured_glyph() {
        assert_eq!(IconState::Active.badge(), Some(Badge::Connected));
        assert_eq!(IconState::Attention.badge(), Some(Badge::Attention));
        assert_eq!(IconState::Sync.badge(), Some(Badge::Sync));
        assert_eq!(IconState::Plain.badge(), None);
        assert_eq!(IconState::Dim.badge(), None);
        for state in [IconState::Plain, IconState::Sync, IconState::Attention, IconState::Dim, IconState::Active] {
            assert_eq!(state.icon_name(), "yutani-symbolic", "{state:?}: one shape, one name");
            let file = format!("{}.svg", state.icon_name());
            assert!(crate::assets::ICONS.iter().any(|(_, n, _)| *n == file), "{file}");
            assert_eq!(state.opacity(), if state == IconState::Dim { 0.4 } else { 1.0 });
        }
    }

    /// The slice through the stem lands on half a pixel at 16 px, so the
    /// panel gets the solid mark below 22 and the two-piece mark from there.
    #[test]
    fn the_solid_mark_is_used_below_22_px() {
        for px in [8u16, 16, 21] {
            assert_eq!(IconState::Plain.bytes(px), crate::assets::YUTANI_SYMBOLIC_16, "{px}");
        }
        for px in [22u16, 24, 32, 64] {
            assert_eq!(IconState::Active.bytes(px), crate::assets::YUTANI_SYMBOLIC, "{px}");
        }
    }

    /// A broken Steam launch line is the same kind of news as a failed
    /// unit: Yutani is running and something the user set up is wrong.
    /// It outranks every live state but not the dim mark, which already
    /// says there is nothing to run through.
    #[test]
    fn a_steam_problem_demands_attention_unless_the_mark_is_dim() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), 0, false, true), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), 2, false, true), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, None)), 0, false, true), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), 0, true, true), IconState::Attention);
        assert_eq!(icon_state(None, 0, false, true), IconState::Dim);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), 0, false, true), IconState::Dim);
    }
}
