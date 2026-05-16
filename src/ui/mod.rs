pub mod bulk_rename;
pub mod file_grid;
pub mod main_window;
pub mod modal_host;
pub mod ops_panel;
pub mod preview_pane;
pub mod search_panel;
pub mod sidebar;
pub mod status_bar;
pub mod tab_strip;
pub mod tag_filter;
pub mod toolbar;

use glib::SourceId;
use gtk::prelude::*;
use gtk::{Box as GtkBox, EventControllerMotion, GestureClick, Label, Orientation, Popover};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const TOOLTIP_DELAY_MS: u64 = 350;
const TOOLTIP_MAX_WIDTH_CHARS: i32 = 56;

fn cancel_tooltip_timer(timer: &Rc<RefCell<Option<SourceId>>>) {
    if let Some(source_id) = timer.borrow_mut().take() {
        source_id.remove();
    }
}

pub fn tooltip_host<W>(child: &W, text: impl Into<String>) -> GtkBox
where
    W: IsA<gtk::Widget>,
{
    let host = GtkBox::new(Orientation::Horizontal, 0);
    host.add_css_class("tooltip-host");
    host.append(child);
    attach_tooltip(&host, text);
    host
}

/// Attaches a deterministic hover tooltip using a popover instead of GTK's
/// built-in tooltip widget, which has been rendering with unstable sizing.
pub fn attach_tooltip<W>(widget: &W, text: impl Into<String>)
where
    W: IsA<gtk::Widget> + Clone + 'static,
{
    let widget: gtk::Widget = widget.clone().upcast();

    let frame = GtkBox::new(Orientation::Horizontal, 0);
    frame.add_css_class("app-tooltip-frame");

    let label = Label::new(Some(&text.into()));
    label.add_css_class("app-tooltip-label");
    label.set_single_line_mode(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(TOOLTIP_MAX_WIDTH_CHARS);
    frame.append(&label);

    let popover = Popover::new();
    popover.add_css_class("app-tooltip-popover");
    popover.set_has_arrow(false);
    popover.set_autohide(false);
    popover.set_can_target(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_child(Some(&frame));
    popover.set_parent(&widget);

    let hover_timer = Rc::new(RefCell::new(None::<SourceId>));

    let motion = EventControllerMotion::new();
    {
        let hover_timer = Rc::clone(&hover_timer);
        let popover = popover.clone();
        let widget = widget.clone();
        motion.connect_enter(move |_, _, _| {
            cancel_tooltip_timer(&hover_timer);

            let hover_timer_for_timeout = Rc::clone(&hover_timer);
            let popover = popover.clone();
            let widget = widget.clone();
            *hover_timer.borrow_mut() = Some(glib::timeout_add_local_once(
                Duration::from_millis(TOOLTIP_DELAY_MS),
                move || {
                    hover_timer_for_timeout.borrow_mut().take();
                    if widget.is_visible() {
                        popover.popup();
                    }
                },
            ));
        });
    }
    {
        let hover_timer = Rc::clone(&hover_timer);
        let popover = popover.clone();
        motion.connect_leave(move |_| {
            cancel_tooltip_timer(&hover_timer);
            popover.popdown();
        });
    }
    widget.add_controller(motion);

    let click = GestureClick::new();
    {
        let hover_timer = Rc::clone(&hover_timer);
        let popover = popover.clone();
        click.connect_pressed(move |_, _, _, _| {
            cancel_tooltip_timer(&hover_timer);
            popover.popdown();
        });
    }
    widget.add_controller(click);
}
