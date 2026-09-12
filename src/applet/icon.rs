//! Which panel icon the tunnel's state calls for (spec §3).

use crate::tunnel::status::TunnelStatus;

/// A tunnel that is up but has not handshaked for this long is in trouble.
pub const HANDSHAKE_STALE_S: u64 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    /// The plain mark: connected and healthy, or deliberately disconnected.
    Plain,
    /// Ring badge: an action is in flight, or the tunnel is up and still
    /// handshaking.
    Sync,
    /// Exclamation badge: up but no handshake for `HANDSHAKE_STALE_S`.
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
            IconState::Plain | IconState::Dim => "y-symbolic",
        }
    }

    pub fn opacity(self) -> f32 {
        if self == IconState::Dim { super::theme::DIM_OPACITY } else { 1.0 }
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            IconState::Sync => crate::assets::Y_SYNC_SYMBOLIC,
            IconState::Attention => crate::assets::Y_ATTENTION_SYMBOLIC,
            IconState::Plain | IconState::Dim => crate::assets::Y_SYMBOLIC,
        }
    }
}

/// `tunnel` is `None` when the daemon did not answer. `pending` is true for
/// [`super::PENDING_S`] after a `tunnel connect|disconnect` was sent.
pub fn icon_state(tunnel: Option<&TunnelStatus>, pending: bool) -> IconState {
    let Some(t) = tunnel else { return IconState::Dim };
    if !t.installed {
        return IconState::Dim;
    }
    if pending {
        return IconState::Sync;
    }
    match (t.connected, t.handshake_age_s) {
        (false, _) => IconState::Plain,
        (true, None) => IconState::Sync,
        (true, Some(age)) if age >= HANDSHAKE_STALE_S => IconState::Attention,
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
            rx_bytes: 0,
            tx_bytes: 0,
        }
    }

    #[test]
    fn no_daemon_and_no_tunnel_are_both_dim() {
        assert_eq!(icon_state(None, false), IconState::Dim);
        assert_eq!(icon_state(Some(&tunnel(false, false, None)), false), IconState::Dim);
        // Even a pending action cannot brighten a missing daemon.
        assert_eq!(icon_state(None, true), IconState::Dim);
    }

    #[test]
    fn a_pending_action_shows_the_sync_icon() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), true), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(3))), true), IconState::Sync);
    }

    #[test]
    fn a_deliberate_disconnect_and_a_fresh_handshake_are_both_plain() {
        assert_eq!(icon_state(Some(&tunnel(true, false, None)), false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(0))), false), IconState::Plain);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(179))), false), IconState::Plain);
    }

    #[test]
    fn up_without_a_handshake_syncs_and_a_stale_handshake_demands_attention() {
        assert_eq!(icon_state(Some(&tunnel(true, true, None)), false), IconState::Sync);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(180))), false), IconState::Attention);
        assert_eq!(icon_state(Some(&tunnel(true, true, Some(6_000))), false), IconState::Attention);
    }

    #[test]
    fn each_state_names_one_of_the_installed_icons() {
        assert_eq!(IconState::Plain.icon_name(), "y-symbolic");
        assert_eq!(IconState::Sync.icon_name(), "y-sync-symbolic");
        assert_eq!(IconState::Attention.icon_name(), "y-attention-symbolic");
        assert_eq!(IconState::Dim.icon_name(), "y-symbolic");
        assert_eq!(IconState::Plain.opacity(), 1.0);
        assert_eq!(IconState::Dim.opacity(), 0.38);
    }
}
