//! The COSMIC-theme look: the panel button's tinted mark and status badge,
//! and the theme roles the settings window draws with. The popover's own
//! look is `skin.rs`, fixed and never read from the theme.

use cosmic::iced::border::Radius;
use cosmic::iced::widget::container;
use cosmic::iced::{Background, Border, Color};

/// The status badge's box (ring included) for a panel icon `icon_px`
/// tall: a third of the icon, 7–10 px. The handoff's 7×7 is drawn for a
/// 24 px icon at 2×; a fixed 10 px box (7 + the ring) covered most of the
/// mark on Daniel's small panel (2026-10-07), so it scales as the round
/// dot before it did.
pub fn badge_px(icon_px: f32) -> f32 {
    (icon_px / 3.0).round().clamp(7.0, 10.0)
}
/// The status badge's ring, in the panel's own background.
pub const BADGE_RING_PX: f32 = 1.5;
/// The panel icon's opacity in the daemon-offline / not-installed state:
/// the redesign's 40 %, "present but inactive".
pub const DIM_OPACITY: f32 = 0.40;

/// Disabled text, as a factor on its colour's alpha.
pub const DISABLED_ALPHA: f32 = 0.45;

// ---- theme roles ----------------------------------------------------------

pub fn with_alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}

pub fn dimmed(color: Color) -> Color {
    Color { a: color.a * DISABLED_ALPHA, ..color }
}

type Cosmic = cosmic::cosmic_theme::Theme;

/// Text primary.
pub fn ink(c: &Cosmic) -> Color {
    c.on_bg_color().into()
}
/// Text secondary: `Text::Default` at 70 %.
pub fn secondary(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.70)
}
/// Text tertiary / captions: `Text::Default` at 55 %.
pub fn tertiary(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.55)
}
pub fn success(c: &Cosmic) -> Color {
    c.success_color().into()
}
pub fn accent(c: &Cosmic) -> Color {
    c.accent_color().into()
}
pub fn destructive(c: &Cosmic) -> Color {
    c.destructive_color().into()
}
pub fn warning(c: &Cosmic) -> Color {
    c.warning_color().into()
}
/// Card / list surface.
pub fn card_bg(c: &Cosmic) -> Color {
    c.background(false).component.base.into()
}
/// Card edge.
pub fn card_border(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.10)
}
/// Row divider.
pub fn divider(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.08)
}
/// Control background and border.
pub fn control_bg(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.07)
}
pub fn control_border(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.14)
}
/// Row hover.
pub fn hover(c: &Cosmic) -> Color {
    with_alpha(ink(c), 0.09)
}
/// The panel background, for the status badge's cut-out ring.
pub fn panel_bg(c: &Cosmic) -> Color {
    c.bg_color().into()
}

/// The panel mark's ink: pure white on a dark theme (the theme's `on_bg`
/// is a soft grey that reads dull at 16 px), the theme's `on_bg` on a
/// light one.
pub const MARK_ON_DARK: Color = Color::WHITE;

fn mark_ink(c: &Cosmic) -> Color {
    if c.is_dark { MARK_ON_DARK } else { ink(c) }
}

/// The SVG class for the tinted panel mark.
pub fn mark_class() -> cosmic::theme::Svg {
    cosmic::theme::Svg::custom(|theme| cosmic::iced::widget::svg::Style { color: Some(mark_ink(theme.cosmic())) })
}

/// The status badge's fill on a dark panel, and whether it glows.
pub fn badge_fill(badge: super::icon::Badge) -> (Color, bool) {
    use super::icon::Badge;
    match badge {
        Badge::Connected => (crate::applet::skin::PHOSPHOR, true),
        Badge::Attention | Badge::Sync => (crate::applet::skin::AMBER, false),
    }
}

/// The status badge: phosphor (glowing) or amber on a dark panel, the
/// theme's success/warning on a light one (phosphor on a light grey is
/// under 2:1); a 1.5 px ring in the panel's own colour; Sync is a hollow
/// outline.
pub fn badge_class(badge: super::icon::Badge) -> cosmic::theme::Container<'static> {
    use super::icon::Badge;
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        let panel = panel_bg(c);
        let (fill, glow) = if c.is_dark {
            badge_fill(badge)
        } else {
            (if badge == Badge::Connected { success(c) } else { warning(c) }, false)
        };
        let (bg, ring) = if badge == Badge::Sync { (panel, fill) } else { (fill, panel) };
        container::Style {
            background: Some(Background::Color(bg)),
            border: Border { radius: Radius::from(1.0), width: BADGE_RING_PX, color: ring },
            shadow: if glow { cosmic::iced::Shadow { color: Color { a: 0.5, ..fill }, offset: cosmic::iced::Vector::ZERO, blur_radius: 3.0 } } else { Default::default() },
            ..Default::default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A third of the icon, never below 7 or above 10.
    #[test]
    fn the_badge_scales_with_the_icon() {
        assert_eq!(badge_px(24.0), 8.0);
        assert_eq!(badge_px(16.0), 7.0);
        assert_eq!(badge_px(64.0), 10.0);
        assert_eq!(BADGE_RING_PX, 1.5);
        assert_eq!(DIM_OPACITY, 0.40);
        assert_eq!(badge_fill(crate::applet::icon::Badge::Connected), (crate::applet::skin::PHOSPHOR, true));
        assert_eq!(badge_fill(crate::applet::icon::Badge::Attention), (crate::applet::skin::AMBER, false));
    }

    #[test]
    fn alpha_helpers_touch_only_alpha() {
        let c = with_alpha(Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 }, 0.5);
        assert_eq!((c.r, c.g, c.b, c.a), (0.1, 0.2, 0.3, 0.5));
        assert!((dimmed(c).a - 0.5 * DISABLED_ALPHA).abs() < 1e-6);
    }
}
