//! The panel button. The popover is `console_view`.

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::Length;
use cosmic::widget;
use cosmic::Element;

use yutani::applet::icon::icon_state;
use yutani::applet::theme;

use crate::app::{Applet, Msg, close_popup_message, open_popup_message};

/// The panel button: the mark, tinted by the panel, with the state's badge
/// over its bottom-right corner (`IconState::badge`).
pub fn panel_button(state: &Applet) -> Element<'_, Msg> {
    let clients = state.status.as_ref().map_or(0, |s| s.clients.len());
    let steam_problem = state.status.as_ref().is_some_and(|s| yutani::steam::first_message(&s.steam).is_some());
    let icon = icon_state(state.status.as_ref().map(|s| &s.tunnel), clients, state.pending(), steam_problem);
    let (w, h) = state.core.applet.suggested_size(true);
    let mark = widget::icon(widget::icon::from_svg_bytes(icon.bytes(h)).symbolic(true))
        .class(theme::mark_class())
        .width(Length::Fixed(f32::from(w)))
        .height(Length::Fixed(f32::from(h)))
        .opacity(icon.opacity());
    let content: Element<'_, Msg> = if let Some(badge) = icon.badge() {
        let d = theme::badge_px(f32::from(h));
        let badge = widget::container(widget::space().width(Length::Fixed(d)).height(Length::Fixed(d)))
            .class(theme::badge_class(badge));
        cosmic::iced::widget::stack([
            mark.into(),
            widget::container(badge).width(Length::Fill).height(Length::Fill).align_x(Horizontal::Right).align_y(Vertical::Bottom).into(),
        ])
        .into()
    } else {
        mark.into()
    };
    let open = state.popup;
    state
        .core
        .applet
        .button_from_element(content, true)
        .on_press_with_rectangle(move |offset, bounds| match open {
            Some(id) => close_popup_message(id),
            None => open_popup_message(bounds, offset),
        })
        .into()
}
