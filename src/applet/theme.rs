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
/// The panel icon's opacity in the daemon-offline state:
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

/// The EVE badge's blue on a dark panel. COSMIC's theme has no blue role
/// (its accent is the user's choice), so it is fixed.
pub const EVE_BADGE_ON_DARK: Color = Color::from_rgb8(0x4d, 0xa6, 0xff);
/// The same blue darkened for a light panel.
pub const EVE_BADGE_ON_LIGHT: Color = Color::from_rgb8(0x1f, 0x6f, 0xd8);

/// The status badge's fill on a dark panel. No glow: Daniel asked for a
/// plain dot in the tray (2026-10-07).
pub fn badge_fill(badge: super::icon::Badge) -> Color {
    use super::icon::Badge;
    match badge {
        Badge::Eve => EVE_BADGE_ON_DARK,
        Badge::Attention => crate::applet::skin::AMBER,
    }
}

/// The status badge: blue or amber on a dark panel, a darker blue or the
/// theme's warning on a light one; a 1.5 px ring in the panel's own colour.
pub fn badge_class(badge: super::icon::Badge) -> cosmic::theme::Container<'static> {
    use super::icon::Badge;
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        let panel = panel_bg(c);
        let fill = if c.is_dark {
            badge_fill(badge)
        } else {
            match badge {
                Badge::Eve => EVE_BADGE_ON_LIGHT,
                Badge::Attention => warning(c),
            }
        };
        container::Style {
            background: Some(Background::Color(fill)),
            border: Border { radius: Radius::from(1.0), width: BADGE_RING_PX, color: panel },
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
        assert_eq!(badge_fill(crate::applet::icon::Badge::Attention), crate::applet::skin::AMBER);
        assert_eq!(badge_fill(crate::applet::icon::Badge::Eve), EVE_BADGE_ON_DARK);
    }

    #[test]
    fn alpha_helpers_touch_only_alpha() {
        let c = with_alpha(Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 }, 0.5);
        assert_eq!((c.r, c.g, c.b, c.a), (0.1, 0.2, 0.3, 0.5));
        assert!((dimmed(c).a - 0.5 * DISABLED_ALPHA).abs() < 1e-6);
    }
}
