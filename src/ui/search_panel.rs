use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Entry, Label, Orientation, ToggleButton};
use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchKindFilter {
    All,
    Folders,
    Images,
    Videos,
    Text,
    Archives,
    Code,
}

impl SearchKindFilter {
    pub const ALL: &'static [Self] = &[
        Self::All,
        Self::Folders,
        Self::Images,
        Self::Videos,
        Self::Text,
        Self::Archives,
        Self::Code,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "All Types",
            Self::Folders => "Folders",
            Self::Images => "Images",
            Self::Videos => "Videos",
            Self::Text => "Text",
            Self::Archives => "Archives",
            Self::Code => "Code",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchAgeFilter {
    Any,
    Today,
    ThisWeek,
    ThisMonth,
    Older,
}

impl SearchAgeFilter {
    pub const ALL: &'static [Self] = &[
        Self::Any,
        Self::Today,
        Self::ThisWeek,
        Self::ThisMonth,
        Self::Older,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Self::Any => "Any Date",
            Self::Today => "Today",
            Self::ThisWeek => "This Week",
            Self::ThisMonth => "This Month",
            Self::Older => "Older",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchSizeFilter {
    Any,
    Small,
    Medium,
    Large,
}

impl SearchSizeFilter {
    pub const ALL: &'static [Self] = &[Self::Any, Self::Small, Self::Medium, Self::Large];
    pub fn label(&self) -> &'static str {
        match self {
            Self::Any => "Any Size",
            Self::Small => "< 1 MB",
            Self::Medium => "1–50 MB",
            Self::Large => "> 50 MB",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    pub name: String,
    pub kind: SearchKindFilter,
    pub age: SearchAgeFilter,
    pub size: SearchSizeFilter,
    pub tag_id: Option<i64>,
    pub scope_dir: std::path::PathBuf,
    pub recursive: bool,
}

impl SearchQuery {
    pub fn new(scope_dir: std::path::PathBuf) -> Self {
        Self {
            name: String::new(),
            kind: SearchKindFilter::All,
            age: SearchAgeFilter::Any,
            size: SearchSizeFilter::Any,
            tag_id: None,
            scope_dir,
            recursive: true,
        }
    }
}

#[derive(Clone)]
pub struct SearchPanel {
    pub root: GtkBox,
    pub name_entry: Entry,
    pub recursive_toggle: ToggleButton,
    pub scope_label: Label,
    pub kind_buttons: Vec<(SearchKindFilter, Button)>,
    pub age_buttons: Vec<(SearchAgeFilter, Button)>,
    pub size_buttons: Vec<(SearchSizeFilter, Button)>,
    pub tag_row: GtkBox,
    tag_buttons: RefCell<Vec<(i64, Button)>>,
    /// True while sync_from_query is running; suppresses connect_changed.
    updating: Cell<bool>,
}

impl SearchPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("search-strip");
        root.set_visible(false);

        // ── Row 1: name entry + scope + recurse ──────────────────────
        let row1 = GtkBox::new(Orientation::Horizontal, 6);
        row1.add_css_class("search-row");

        let scope_label = Label::new(Some("in ~"));
        scope_label.add_css_class("search-scope-label");
        scope_label.set_halign(gtk::Align::Start);
        scope_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        scope_label.set_max_width_chars(28);

        let name_entry = Entry::new();
        name_entry.set_placeholder_text(Some("Search by name…"));
        name_entry.add_css_class("search-entry");
        name_entry.set_hexpand(true);

        let recursive_toggle = ToggleButton::with_label("Recurse");
        recursive_toggle.add_css_class("search-chip");
        recursive_toggle.add_css_class("search-chip-toggle");
        recursive_toggle.set_active(true);

        row1.append(&name_entry);
        row1.append(&scope_label);
        row1.append(&recursive_toggle);
        root.append(&row1);

        // ── Row 2: kind chips ─────────────────────────────────────────
        let kind_row = chip_row();
        let mut kind_buttons = Vec::new();
        for filter in SearchKindFilter::ALL {
            let btn = chip_button(filter.label());
            if *filter == SearchKindFilter::All {
                btn.add_css_class("active");
            }
            kind_row.append(&btn);
            kind_buttons.push((filter.clone(), btn));
        }
        root.append(&kind_row);

        // ── Row 3: age + size chips ───────────────────────────────────
        let age_size_row = chip_row();

        let age_sep = Label::new(Some("When:"));
        age_sep.add_css_class("search-chip-label");
        age_size_row.append(&age_sep);

        let mut age_buttons = Vec::new();
        for filter in SearchAgeFilter::ALL {
            let btn = chip_button(filter.label());
            if *filter == SearchAgeFilter::Any {
                btn.add_css_class("active");
            }
            age_size_row.append(&btn);
            age_buttons.push((filter.clone(), btn));
        }

        let size_sep = Label::new(Some("Size:"));
        size_sep.add_css_class("search-chip-label");
        size_sep.set_margin_start(12);
        age_size_row.append(&size_sep);

        let mut size_buttons = Vec::new();
        for filter in SearchSizeFilter::ALL {
            let btn = chip_button(filter.label());
            if *filter == SearchSizeFilter::Any {
                btn.add_css_class("active");
            }
            age_size_row.append(&btn);
            size_buttons.push((filter.clone(), btn));
        }
        root.append(&age_size_row);

        // ── Row 4: tag chips (populated dynamically) ──────────────────
        let tag_row = chip_row();
        tag_row.set_visible(false);
        root.append(&tag_row);

        Self {
            root,
            name_entry,
            recursive_toggle,
            scope_label,
            kind_buttons,
            age_buttons,
            size_buttons,
            tag_row,
            tag_buttons: RefCell::new(Vec::new()),
            updating: Cell::new(false),
        }
    }

    pub fn is_updating(&self) -> bool {
        self.updating.get()
    }

    pub fn sync_from_query(&self, query: &SearchQuery) {
        self.updating.set(true);
        self.name_entry.set_text(&query.name);
        self.recursive_toggle.set_active(query.recursive);

        for (filter, btn) in &self.kind_buttons {
            set_chip_active(btn, filter == &query.kind);
        }
        for (filter, btn) in &self.age_buttons {
            set_chip_active(btn, filter == &query.age);
        }
        for (filter, btn) in &self.size_buttons {
            set_chip_active(btn, filter == &query.size);
        }
        for (id, btn) in self.tag_buttons.borrow().iter() {
            set_chip_active(btn, Some(*id) == query.tag_id);
        }
        self.updating.set(false);
    }

    pub fn set_tags(&self, tags: &[crate::metadata::TagRecord]) {
        clear_box(&self.tag_row);
        let mut buttons = Vec::new();

        if tags.is_empty() {
            self.tag_row.set_visible(false);
            self.tag_buttons.replace(Vec::new());
            return;
        }

        let label = Label::new(Some("Tag:"));
        label.add_css_class("search-chip-label");
        self.tag_row.append(&label);

        let any_btn = chip_button("Any Tag");
        any_btn.add_css_class("active");
        self.tag_row.append(&any_btn);
        buttons.push((-1i64, any_btn));

        for tag in tags {
            let btn = chip_button(&format!("#{}", tag.name));
            self.tag_row.append(&btn);
            buttons.push((tag.id, btn));
        }

        self.tag_row.set_visible(true);
        self.tag_buttons.replace(buttons);
    }

    pub fn tag_buttons(&self) -> Vec<(i64, Button)> {
        self.tag_buttons.borrow().clone()
    }
}

fn chip_row() -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 4);
    row.add_css_class("search-chip-row");
    row
}

fn chip_button(label: &str) -> Button {
    let btn = Button::with_label(label);
    btn.add_css_class("search-chip");
    btn
}

fn set_chip_active(btn: &Button, active: bool) {
    if active {
        btn.add_css_class("active");
    } else {
        btn.remove_css_class("active");
    }
}

fn clear_box(container: &GtkBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
