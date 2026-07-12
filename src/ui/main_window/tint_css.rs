//! Tint colour parsing and CSS generation, split out of main_window.

use crate::metadata::TintRecord;
use gtk::prelude::*;
use gtk::{Adjustment, Align, Box as GtkBox, Label, Orientation, Scale};
use std::cell::RefCell;
use std::rc::Rc;

pub(super) fn parse_hex_rgb(hex: &str) -> (u8, u8, u8) {
    let s = hex.trim().trim_start_matches('#');
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return (128, 96, 64);
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(96);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(64);
    (r, g, b)
}

pub(super) fn rgb_to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub(super) fn build_tint_channel_row(label_text: &str, value: u8) -> (GtkBox, Scale) {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    row.add_css_class("tint-picker-channel-row");

    let label = Label::new(Some(label_text));
    label.add_css_class("tint-picker-channel-label");
    label.set_width_chars(6);
    label.set_halign(Align::End);
    row.append(&label);

    let adjustment = Adjustment::new(value as f64, 0.0, 255.0, 1.0, 16.0, 0.0);
    let scale = Scale::new(Orientation::Horizontal, Some(&adjustment));
    scale.add_css_class("tint-picker-scale");
    scale.set_draw_value(true);
    scale.set_digits(0);
    scale.set_hexpand(true);
    row.append(&scale);

    (row, scale)
}

pub(super) fn wire_tint_channel(
    scale: &Scale,
    state: Rc<RefCell<(u8, u8, u8)>>,
    channel: usize,
    update_ui: Rc<dyn Fn()>,
) {
    scale.connect_value_changed(move |scale| {
        let value = scale.value().round().clamp(0.0, 255.0) as u8;
        {
            let mut rgb = state.borrow_mut();
            match channel {
                0 => rgb.0 = value,
                1 => rgb.1 = value,
                _ => rgb.2 = value,
            }
        }
        update_ui();
    });
}

pub(super) fn generate_tint_css(tints: &[TintRecord]) -> String {
    let mut css = String::new();
    for tint in tints {
        let color = tint.color.as_deref().unwrap_or("#806040");
        // Icon card: tint glow is interactive only. Resting cards should stay
        // visually quiet; hover/selection reveal the Mark's tint.
        let (hover_ring_a, hover_glow_a) = if tint.is_default {
            ("28", "14")
        } else {
            ("58", "44")
        };
        css.push_str(&format!(
            ".file-card-shell:hover > .file-card.mark-tint-{id} {{ box-shadow: 0 0 0 1.5px {c}{hover_ring}, 0 4px 18px 0 {c}{hover_glow}; }}\n",
            id = tint.id, c = color, hover_ring = hover_ring_a, hover_glow = hover_glow_a,
        ));
        css.push_str(&format!(
            "flowboxchild:selected > .file-card-shell > .file-card.mark-tint-{id} {{ box-shadow: 0 0 0 1.5px {c}{hover_ring}, 0 4px 18px 0 {c}{hover_glow}; }}\n",
            id = tint.id, c = color, hover_ring = hover_ring_a, hover_glow = hover_glow_a,
        ));
        // List row: inset left accent via box-shadow on the inner row
        let list_a = if tint.is_default { "22" } else { "58" };
        css.push_str(&format!(
            ".file-list > row.mark-tint-{id} > .file-list-row-inner {{ border-left-color: {c}{list_a}; }}\n",
            id = tint.id, c = color, list_a = list_a,
        ));
    }
    css
}
