//! Which panel icon the daemon's state calls for (redesign spec §1).
//!
//! The glyph is never recoloured: every state is the panel's own ink, at
//! full strength or at 40 %, and the news is a dot in the bottom-right
//! corner — blue while EVE runs, the warning colour for a broken Steam
//! launch line.

/// The panel icon: the solid mark below this many pixels, the two-piece
/// mark (with the slice through the stem) from here up. The handoff's
/// threshold is 22 px on the 32-unit grid; the panel marks are cropped to
/// 25 units (`assets::PANEL_MARK`), so the same slice width arrives at
/// 22 × 25/32 ≈ 17.2 px. On Daniel's 20 px panel the slice is ~1.3 px.
pub const TWO_PIECE_MIN_PX: u16 = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    /// The plain mark: the daemon runs, no EVE client yet.
    Plain,
    /// The mark with the blue dot: an EVE client is running.
    Eve,
    /// The mark with the warning dot: a Steam launch line is broken.
    Attention,
    /// The mark at 40 %: no daemon.
    Dim,
}

/// What is drawn over the mark's bottom-right corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// Filled, blue (`theme::EVE_BADGE_ON_DARK`).
    Eve,
    /// Filled, the theme's warning colour.
    Attention,
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
            IconState::Eve => Some(Badge::Eve),
            IconState::Attention => Some(Badge::Attention),
            IconState::Plain | IconState::Dim => None,
        }
    }

    /// The mark's bytes for a panel icon `icon_px` tall: the solid variant
    /// below [`TWO_PIECE_MIN_PX`], where the slice would land on half a
    /// pixel, the two-piece mark otherwise.
    pub fn bytes(self, icon_px: u16) -> &'static [u8] {
        if icon_px < TWO_PIECE_MIN_PX { crate::assets::PANEL_MARK_SOLID } else { crate::assets::PANEL_MARK }
    }
}

/// `running` is whether the daemon answered. `clients` is how many EVE
/// clients it is tracking — any at all is the blue [`IconState::Eve`] dot.
/// `steam_problem` is true when the daemon found a Steam account whose EVE
/// launch line is broken (`Status::steam`): the warning dot, unless the
/// mark is dim anyway.
pub fn icon_state(running: bool, clients: usize, steam_problem: bool) -> IconState {
    if !running {
        IconState::Dim
    } else if steam_problem {
        IconState::Attention
    } else if clients > 0 {
        IconState::Eve
    } else {
        IconState::Plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_daemon_is_dim_whatever_else_is_true() {
        assert_eq!(icon_state(false, 0, false), IconState::Dim);
        assert_eq!(icon_state(false, 3, true), IconState::Dim);
    }

    #[test]
    fn the_blue_dot_is_any_eve_client() {
        assert_eq!(icon_state(true, 0, false), IconState::Plain);
        assert_eq!(icon_state(true, 1, false), IconState::Eve);
        assert_eq!(icon_state(true, 9, false), IconState::Eve);
    }

    /// Redesign spec §1: the glyph is never recoloured. Status is a corner
    /// badge — blue or filled warning — and the stopped state is the same
    /// shape at 40 %.
    #[test]
    fn status_is_a_corner_badge_and_never_a_recoloured_glyph() {
        assert_eq!(IconState::Eve.badge(), Some(Badge::Eve));
        assert_eq!(IconState::Attention.badge(), Some(Badge::Attention));
        assert_eq!(IconState::Plain.badge(), None);
        assert_eq!(IconState::Dim.badge(), None);
        for state in [IconState::Plain, IconState::Attention, IconState::Dim, IconState::Eve] {
            assert_eq!(state.icon_name(), "yutani-symbolic", "{state:?}: one shape, one name");
            let file = format!("{}.svg", state.icon_name());
            assert!(crate::assets::ICONS.iter().any(|(_, n, _)| *n == file), "{file}");
            assert_eq!(state.opacity(), if state == IconState::Dim { 0.4 } else { 1.0 });
        }
    }

    /// The slice through the stem lands on half a pixel at 16 px, so the
    /// panel gets the solid mark below 18 and the two-piece mark from there.
    #[test]
    fn the_solid_mark_is_used_below_18_px() {
        for px in [8u16, 16, 17] {
            assert_eq!(IconState::Plain.bytes(px), crate::assets::PANEL_MARK_SOLID, "{px}");
        }
        for px in [18u16, 20, 24, 32, 64] {
            assert_eq!(IconState::Eve.bytes(px), crate::assets::PANEL_MARK, "{px}");
        }
    }

    /// A broken Steam launch line is news the user must see before
    /// pressing Play: it outranks the blue dot, but not the dim mark, which
    /// already says there is nothing to run through.
    #[test]
    fn a_steam_problem_demands_attention_unless_the_mark_is_dim() {
        assert_eq!(icon_state(true, 0, true), IconState::Attention);
        assert_eq!(icon_state(true, 2, true), IconState::Attention);
        assert_eq!(icon_state(false, 0, true), IconState::Dim);
    }
}
