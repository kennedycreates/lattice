use gtk::prelude::*;
use gtk::{Box, Button, Orientation};

#[derive(Clone)]
pub struct TabStrip {
    pub root: Box,
    pub tabs_box: Box,
    pub new_tab_button: Button,
}

impl TabStrip {
    pub fn build() -> Self {
        let root = Box::new(Orientation::Horizontal, 8);
        root.add_css_class("tab-strip");

        let tabs_box = Box::new(Orientation::Horizontal, 6);
        tabs_box.set_hexpand(true);
        root.append(&tabs_box);

        let new_tab_button = Button::with_label("+");
        new_tab_button.add_css_class("tab-add-button");
        new_tab_button.set_tooltip_text(Some("Open a new tab"));
        root.append(&new_tab_button);

        Self {
            root,
            tabs_box,
            new_tab_button,
        }
    }
}
