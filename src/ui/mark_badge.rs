use crate::metadata::Shape;
use gtk::cairo;
use gtk::prelude::*;
use gtk::DrawingArea;

pub(crate) fn make_shape_badge(shape: Shape, size: i32, tint_color: Option<&str>) -> DrawingArea {
    let area = DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.add_css_class("file-mark-shape-badge");
    let (fill_r, fill_g, fill_b) = tint_color
        .and_then(parse_hex_rgb)
        .unwrap_or((0.88, 0.76, 0.54));
    area.set_draw_func(move |_, cr, w, h| {
        let cx = w as f64 / 2.0;
        let cy = h as f64 / 2.0;
        let r = (w.min(h) as f64 / 2.0) * 0.80;
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
        draw_shape_path(cr, shape, cx, cy, r + 1.2);
        let _ = cr.fill();
        cr.set_source_rgba(fill_r, fill_g, fill_b, 0.95);
        draw_shape_path(cr, shape, cx, cy, r);
        let _ = cr.fill();
    });
    area
}

fn parse_hex_rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let s = hex.trim().trim_start_matches('#');
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}

fn draw_shape_path(cr: &cairo::Context, shape: Shape, cx: f64, cy: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    match shape {
        Shape::Circle => {
            cr.arc(cx, cy, r, 0.0, 2.0 * PI);
        }
        Shape::Square => {
            cr.rectangle(cx - r, cy - r, r * 2.0, r * 2.0);
        }
        Shape::Triangle => {
            let offset = r * 0.12;
            cr.move_to(cx, cy - r + offset);
            cr.line_to(cx + r * 0.866, cy + r * 0.5 + offset);
            cr.line_to(cx - r * 0.866, cy + r * 0.5 + offset);
            cr.close_path();
        }
        Shape::Pentagon => draw_regular_polygon(cr, cx, cy, r, 5, FRAC_PI_2),
        Shape::Hexagon => draw_regular_polygon(cr, cx, cy, r, 6, FRAC_PI_2),
        Shape::Octagon => draw_regular_polygon(cr, cx, cy, r, 8, FRAC_PI_2 - PI / 8.0),
        Shape::Trapezoid => {
            let top = r * 0.62;
            cr.move_to(cx - top, cy - r * 0.45);
            cr.line_to(cx + top, cy - r * 0.45);
            cr.line_to(cx + r, cy + r * 0.55);
            cr.line_to(cx - r, cy + r * 0.55);
            cr.close_path();
        }
    }
}

fn draw_regular_polygon(cr: &cairo::Context, cx: f64, cy: f64, r: f64, n: i32, start: f64) {
    for i in 0..n {
        let angle = start + 2.0 * std::f64::consts::PI * i as f64 / n as f64;
        let x = cx + r * angle.cos();
        let y = cy - r * angle.sin();
        if i == 0 {
            cr.move_to(x, y);
        } else {
            cr.line_to(x, y);
        }
    }
    cr.close_path();
}
