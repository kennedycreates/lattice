use crate::metadata::{Shape, TagRecord, TintRecord};
use crate::ui::file_grid::{FileItem, FileKind};
use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, ListBox, ListBoxRow,
    Orientation, Revealer, RevealerTransitionType, ScrolledWindow, StringList,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KindFilter {
    All,
    Folders,
    Images,
    Videos,
    Audio,
    Documents,
    Archives,
    Code,
}

impl KindFilter {
    const ALL: &'static [Self] = &[
        Self::All,
        Self::Folders,
        Self::Images,
        Self::Videos,
        Self::Audio,
        Self::Documents,
        Self::Archives,
        Self::Code,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Folders => "Folders",
            Self::Images => "Images",
            Self::Videos => "Videos",
            Self::Audio => "Audio",
            Self::Documents => "Docs",
            Self::Archives => "Archives",
            Self::Code => "Code",
        }
    }

    fn matches(self, item: &FileItem) -> bool {
        match self {
            Self::All => true,
            Self::Folders => item.is_dir,
            Self::Images => item.kind == FileKind::Image,
            Self::Videos => item.kind == FileKind::Video,
            Self::Audio => item.kind == FileKind::Audio,
            Self::Documents => matches!(
                item.kind,
                FileKind::Document | FileKind::Text | FileKind::ConfigCode
            ),
            Self::Archives => item.kind == FileKind::Archive,
            Self::Code => item.kind == FileKind::ConfigCode,
        }
    }
}

#[derive(Clone)]
struct RowWidgets {
    path: PathBuf,
    entry: Entry,
    status: Label,
    row: ListBoxRow,
}

struct State {
    items: RefCell<Vec<FileItem>>,
    sibling_names: RefCell<HashMap<PathBuf, HashSet<String>>>,
    edits: RefCell<HashMap<PathBuf, String>>,
    rows: RefCell<Vec<RowWidgets>>,
    recursive_toggle: CheckButton,
    name_filter: Entry,
    kind_filter: RefCell<KindFilter>,
    tint_filter: RefCell<Option<i64>>,
    shape_filter: RefCell<Option<Shape>>,
    tag_filter: RefCell<Option<i64>>,
    kind_dropdown: DropDown,
    tint_dropdown: DropDown,
    tint_ids: RefCell<Vec<Option<i64>>>,
    shape_dropdown: DropDown,
    shape_ids: RefCell<Vec<Option<Shape>>>,
    tag_dropdown: DropDown,
    tag_ids: RefCell<Vec<Option<i64>>>,
    rows_list: ListBox,
    summary_label: Label,
    apply_button: Button,
    find_entry: Entry,
    replace_entry: Entry,
    prefix_entry: Entry,
    suffix_entry: Entry,
    on_apply: RefCell<Option<Box<dyn Fn(Vec<(PathBuf, String)>)>>>,
    on_refresh: RefCell<Option<Box<dyn Fn(bool)>>>,
}

impl State {
    fn refresh_rows(self: &Rc<Self>) {
        while let Some(child) = self.rows_list.first_child() {
            self.rows_list.remove(&child);
        }
        self.rows.borrow_mut().clear();

        let items = self.items.borrow().clone();
        let visible = items
            .into_iter()
            .filter(|item| self.matches_filters(item))
            .collect::<Vec<_>>();

        if visible.is_empty() {
            let empty = Label::new(Some("No files match the current naming filters."));
            empty.add_css_class("bn-empty");
            empty.set_halign(Align::Start);
            empty.set_margin_top(16);
            empty.set_margin_bottom(16);
            let row = ListBoxRow::new();
            row.add_css_class("bn-row");
            row.set_selectable(false);
            row.set_child(Some(&empty));
            self.rows_list.append(&row);
        } else {
            for item in visible {
                self.add_row(item);
            }
        }
        self.validate();
    }

    fn matches_filters(&self, item: &FileItem) -> bool {
        let name_filter = self.name_filter.text().to_ascii_lowercase();
        if !name_filter.trim().is_empty() && !item.name.to_ascii_lowercase().contains(&name_filter)
        {
            return false;
        }
        if !self.kind_filter.borrow().matches(item) {
            return false;
        }
        if let Some(tint_id) = *self.tint_filter.borrow() {
            if item.mark_tint_id != tint_id {
                return false;
            }
        }
        if let Some(shape) = *self.shape_filter.borrow() {
            if item.mark_shape != shape {
                return false;
            }
        }
        if let Some(tag_id) = *self.tag_filter.borrow() {
            if !item.tags.iter().any(|tag| tag.id == tag_id) {
                return false;
            }
        }
        true
    }

    fn add_row(self: &Rc<Self>, item: FileItem) {
        let row = ListBoxRow::new();
        row.add_css_class("bn-row");
        row.set_selectable(false);

        let body = GtkBox::new(Orientation::Horizontal, 10);
        body.add_css_class("bn-row-body");

        let mark = Label::new(Some(&format!(
            "{} {}",
            item.mark_shape.glyph(),
            item.kind.badge()
        )));
        mark.add_css_class("bn-row-mark");
        mark.set_width_chars(4);
        body.append(&mark);

        let current = Label::new(Some(&item.name));
        current.add_css_class("bn-current-name");
        current.set_halign(Align::Start);
        current.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        current.set_hexpand(true);
        body.append(&current);

        let entry = Entry::new();
        entry.add_css_class("bn-name-entry");
        entry.set_hexpand(true);
        let edited = self
            .edits
            .borrow()
            .get(&item.path)
            .cloned()
            .unwrap_or_else(|| item.name.clone());
        entry.set_text(&edited);
        body.append(&entry);

        let status = Label::new(None);
        status.add_css_class("bn-row-status");
        status.set_width_chars(18);
        status.set_halign(Align::Start);
        body.append(&status);

        row.set_child(Some(&body));
        self.rows_list.append(&row);

        let path = item.path.clone();
        let state = Rc::clone(self);
        entry.connect_changed(move |entry| {
            state
                .edits
                .borrow_mut()
                .insert(path.clone(), entry.text().to_string());
            state.validate();
        });

        self.rows.borrow_mut().push(RowWidgets {
            path: item.path,
            entry,
            status,
            row,
        });
    }

    fn validate(&self) {
        let mut conflicts: HashMap<PathBuf, String> = HashMap::new();
        let mut changed = 0usize;
        let items = self.items.borrow();
        let edits = self.edits.borrow();
        let sibling_names = self.sibling_names.borrow();
        let mut targets: HashMap<(PathBuf, String), Vec<PathBuf>> = HashMap::new();

        for item in items.iter() {
            let new_name = edits
                .get(&item.path)
                .map(String::as_str)
                .unwrap_or(&item.name);
            if new_name == item.name {
                continue;
            }
            changed += 1;
            if let Some(reason) = invalid_name_reason(new_name) {
                conflicts.insert(item.path.clone(), reason.to_string());
                continue;
            }
            let parent = item
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("/"));
            if sibling_names
                .get(&parent)
                .is_some_and(|names| names.contains(new_name))
            {
                conflicts.insert(item.path.clone(), "Name exists".to_string());
                continue;
            }
            targets
                .entry((parent, new_name.to_string()))
                .or_default()
                .push(item.path.clone());
        }

        for paths in targets.values() {
            if paths.len() > 1 {
                for path in paths {
                    conflicts.insert(path.clone(), "Duplicate target".to_string());
                }
            }
        }

        for row in self.rows.borrow().iter() {
            row.entry.remove_css_class("bn-entry-conflict");
            row.row.remove_css_class("bn-row-conflict");
            if let Some(reason) = conflicts.get(&row.path) {
                row.status.set_label(reason);
                row.status.add_css_class("bn-status-conflict");
                row.entry.add_css_class("bn-entry-conflict");
                row.row.add_css_class("bn-row-conflict");
            } else {
                row.status.remove_css_class("bn-status-conflict");
                let item_name = items
                    .iter()
                    .find(|item| item.path == row.path)
                    .map(|item| item.name.as_str())
                    .unwrap_or("");
                if row.entry.text().as_str() == item_name {
                    row.status.set_label("Unchanged");
                } else {
                    row.status.set_label("Ready");
                }
            }
        }

        if conflicts.is_empty() {
            self.summary_label.set_label(&format!(
                "{changed} rename{} ready.",
                if changed == 1 { "" } else { "s" }
            ));
        } else {
            self.summary_label.set_label(&format!(
                "{} conflict{} · {changed} changed",
                conflicts.len(),
                if conflicts.len() == 1 { "" } else { "s" }
            ));
        }
        self.apply_button
            .set_sensitive(changed > 0 && conflicts.is_empty());
    }

    fn pending_renames(&self) -> Vec<(PathBuf, String)> {
        let items = self.items.borrow();
        let edits = self.edits.borrow();
        items
            .iter()
            .filter_map(|item| {
                let new_name = edits.get(&item.path)?;
                if new_name == &item.name || invalid_name_reason(new_name).is_some() {
                    None
                } else {
                    Some((item.path.clone(), new_name.clone()))
                }
            })
            .collect()
    }

    fn apply_to_visible(&self, mut f: impl FnMut(&FileItem, usize) -> String) {
        let visible = self
            .items
            .borrow()
            .iter()
            .filter(|item| self.matches_filters(item))
            .cloned()
            .collect::<Vec<_>>();
        for (index, item) in visible.iter().enumerate() {
            self.edits
                .borrow_mut()
                .insert(item.path.clone(), f(item, index));
        }
    }
}

#[derive(Clone)]
pub struct BulkNamingPanel {
    pub root: GtkBox,
    state: Rc<State>,
}

impl BulkNamingPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("bn-panel");
        root.set_vexpand(true);
        root.set_hexpand(true);

        // Single toolbar: recursive toggle, refresh icon, all filters in one row
        let toolbar = GtkBox::new(Orientation::Horizontal, 6);
        toolbar.add_css_class("bn-toolbar");

        let recursive_toggle = CheckButton::with_label("Recursive");
        recursive_toggle.add_css_class("bn-toggle");
        toolbar.append(&recursive_toggle);

        let refresh_button = Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh")
            .build();
        refresh_button.add_css_class("bn-icon-button");
        toolbar.append(&refresh_button);

        let name_filter = Entry::new();
        name_filter.add_css_class("bn-filter-entry");
        name_filter.set_placeholder_text(Some("contains…"));
        name_filter.set_hexpand(true);
        toolbar.append(&name_filter);

        let kind_labels: Vec<&str> = KindFilter::ALL.iter().map(|k| k.label()).collect();
        let kind_dropdown = DropDown::builder()
            .model(&StringList::new(&kind_labels))
            .build();
        kind_dropdown.add_css_class("bn-filter-dropdown");
        toolbar.append(&kind_dropdown);

        let tint_dropdown = DropDown::builder()
            .model(&StringList::new(&["Any"]))
            .build();
        tint_dropdown.add_css_class("bn-filter-dropdown");
        toolbar.append(&tint_dropdown);

        let shape_dropdown = DropDown::builder()
            .model(&StringList::new(&["Any Shape"]))
            .build();
        shape_dropdown.add_css_class("bn-filter-dropdown");
        toolbar.append(&shape_dropdown);

        let tag_dropdown = DropDown::builder()
            .model(&StringList::new(&["Any Tag"]))
            .build();
        tag_dropdown.add_css_class("bn-filter-dropdown");
        toolbar.append(&tag_dropdown);

        root.append(&toolbar);

        // Collapsible recipe section
        let recipe_header_box = GtkBox::new(Orientation::Horizontal, 0);
        recipe_header_box.add_css_class("bn-recipe-header");
        let recipe_toggle = Button::with_label("▼  Operations");
        recipe_toggle.add_css_class("bn-recipe-toggle");
        recipe_toggle.set_halign(Align::Start);
        recipe_toggle.set_hexpand(true);
        recipe_header_box.append(&recipe_toggle);
        root.append(&recipe_header_box);

        let recipe = GtkBox::new(Orientation::Vertical, 6);
        recipe.add_css_class("bn-recipe-wrap");

        let find_row = GtkBox::new(Orientation::Horizontal, 6);
        let find_entry = recipe_entry("find");
        let replace_entry = recipe_entry("replace");
        let find_btn = recipe_button("Find/Replace");
        find_row.append(&find_entry);
        find_row.append(&replace_entry);
        find_row.append(&find_btn);
        recipe.append(&find_row);

        let affix_row = GtkBox::new(Orientation::Horizontal, 6);
        let prefix_entry = recipe_entry("prefix");
        let prefix_btn = recipe_button("Prefix");
        let suffix_entry = recipe_entry("suffix");
        let suffix_btn = recipe_button("Suffix");
        affix_row.append(&prefix_entry);
        affix_row.append(&prefix_btn);
        affix_row.append(&suffix_entry);
        affix_row.append(&suffix_btn);
        recipe.append(&affix_row);

        let ops_row = GtkBox::new(Orientation::Horizontal, 4);
        ops_row.add_css_class("bn-ops-row");
        let number_btn = ops_chip("Number");
        let case_btn = ops_chip("Clean Case");
        let clear_btn = ops_chip("Clear Changes");
        ops_row.append(&number_btn);
        ops_row.append(&case_btn);
        ops_row.append(&clear_btn);
        recipe.append(&ops_row);

        let recipe_revealer = Revealer::new();
        recipe_revealer.set_transition_type(RevealerTransitionType::SlideDown);
        recipe_revealer.set_reveal_child(true);
        recipe_revealer.set_child(Some(&recipe));
        root.append(&recipe_revealer);

        {
            let recipe_revealer = recipe_revealer.clone();
            recipe_toggle.connect_clicked(move |btn| {
                let revealed = recipe_revealer.reveals_child();
                recipe_revealer.set_reveal_child(!revealed);
                btn.set_label(if revealed {
                    "▶  Operations"
                } else {
                    "▼  Operations"
                });
            });
        }

        let rows_list = ListBox::new();
        rows_list.add_css_class("bn-list");
        rows_list.set_selection_mode(gtk::SelectionMode::None);
        let scroll = ScrolledWindow::builder()
            .child(&rows_list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .build();
        scroll.add_css_class("bn-scroll");
        root.append(&scroll);

        let footer = GtkBox::new(Orientation::Horizontal, 8);
        footer.add_css_class("bn-footer");
        let summary_label = Label::new(Some("No files loaded."));
        summary_label.add_css_class("bn-summary");
        summary_label.set_halign(Align::Start);
        summary_label.set_hexpand(true);
        footer.append(&summary_label);
        let apply_button = Button::with_label("Apply Renames");
        apply_button.add_css_class("bn-apply-button");
        apply_button.set_sensitive(false);
        footer.append(&apply_button);
        root.append(&footer);

        let state = Rc::new(State {
            items: RefCell::new(Vec::new()),
            sibling_names: RefCell::new(HashMap::new()),
            edits: RefCell::new(HashMap::new()),
            rows: RefCell::new(Vec::new()),
            recursive_toggle,
            name_filter,
            kind_filter: RefCell::new(KindFilter::All),
            tint_filter: RefCell::new(None),
            shape_filter: RefCell::new(None),
            tag_filter: RefCell::new(None),
            kind_dropdown,
            tint_dropdown,
            tint_ids: RefCell::new(vec![None]),
            shape_dropdown,
            shape_ids: RefCell::new(vec![None]),
            tag_dropdown,
            tag_ids: RefCell::new(vec![None]),
            rows_list,
            summary_label,
            apply_button,
            find_entry,
            replace_entry,
            prefix_entry,
            suffix_entry,
            on_apply: RefCell::new(None),
            on_refresh: RefCell::new(None),
        });

        wire_static_controls(&state);
        {
            let state = Rc::clone(&state);
            find_btn.connect_clicked(move |_| apply_find_replace(&state));
        }
        {
            let state = Rc::clone(&state);
            prefix_btn.connect_clicked(move |_| {
                let prefix = state.prefix_entry.text().to_string();
                state.apply_to_visible(|item, _| format!("{prefix}{}", item.name));
                state.refresh_rows();
            });
        }
        {
            let state = Rc::clone(&state);
            suffix_btn.connect_clicked(move |_| {
                let suffix = state.suffix_entry.text().to_string();
                state.apply_to_visible(|item, _| append_suffix(&item.name, &suffix, item.is_dir));
                state.refresh_rows();
            });
        }
        {
            let state = Rc::clone(&state);
            number_btn.connect_clicked(move |_| {
                state.apply_to_visible(|item, index| numbered_name(item, index + 1));
                state.refresh_rows();
            });
        }
        {
            let state = Rc::clone(&state);
            case_btn.connect_clicked(move |_| {
                state.apply_to_visible(|item, _| clean_case_name(&item.name, item.is_dir));
                state.refresh_rows();
            });
        }
        {
            let state = Rc::clone(&state);
            clear_btn.connect_clicked(move |_| {
                state.edits.borrow_mut().clear();
                state.refresh_rows();
            });
        }
        {
            let state = Rc::clone(&state);
            refresh_button.connect_clicked(move |_| {
                if let Some(callback) = state.on_refresh.borrow().as_ref() {
                    callback(state.recursive_toggle.is_active());
                }
            });
        }
        {
            let state = Rc::clone(&state);
            let apply_button = state.apply_button.clone();
            apply_button.connect_clicked(move |_| {
                let renames = state.pending_renames();
                if let Some(callback) = state.on_apply.borrow().as_ref() {
                    callback(renames);
                }
            });
        }

        Self { root, state }
    }

    pub fn set_scope(&self, _scope: &Path, recursive: bool, _home: &Path) {
        self.state.recursive_toggle.set_active(recursive);
    }

    pub fn set_loading(&self, message: &str) {
        self.state.items.borrow_mut().clear();
        self.state.edits.borrow_mut().clear();
        self.state.rows.borrow_mut().clear();
        while let Some(child) = self.state.rows_list.first_child() {
            self.state.rows_list.remove(&child);
        }
        self.state.summary_label.set_label(message);
        self.state.apply_button.set_sensitive(false);
    }

    pub fn set_reference_data(&self, tints: &[TintRecord], tags: &[TagRecord]) {
        rebuild_tint_dropdown(&self.state, tints);
        rebuild_shape_dropdown(&self.state);
        rebuild_tag_dropdown(&self.state, tags);
        self.state.refresh_rows();
    }

    pub fn set_items(
        &self,
        items: Vec<FileItem>,
        sibling_names: HashMap<PathBuf, HashSet<String>>,
    ) {
        self.state.items.replace(items);
        self.state.sibling_names.replace(sibling_names);
        self.state.edits.borrow_mut().clear();
        self.state.refresh_rows();
    }

    pub fn recursive_active(&self) -> bool {
        self.state.recursive_toggle.is_active()
    }

    pub fn connect_refresh(&self, callback: impl Fn(bool) + 'static) {
        *self.state.on_refresh.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_apply(&self, callback: impl Fn(Vec<(PathBuf, String)>) + 'static) {
        *self.state.on_apply.borrow_mut() = Some(Box::new(callback));
    }
}

fn wire_static_controls(state: &Rc<State>) {
    {
        let state = Rc::clone(state);
        let name_filter = state.name_filter.clone();
        name_filter.connect_changed(move |_| state.refresh_rows());
    }
    {
        let state = Rc::clone(state);
        let kind_dropdown = state.kind_dropdown.clone();
        kind_dropdown.connect_selected_notify(move |dd| {
            if let Some(&kind) = KindFilter::ALL.get(dd.selected() as usize) {
                *state.kind_filter.borrow_mut() = kind;
                state.refresh_rows();
            }
        });
    }
    {
        let state = Rc::clone(state);
        let tint_dropdown = state.tint_dropdown.clone();
        tint_dropdown.connect_selected_notify(move |dd| {
            let id = state
                .tint_ids
                .borrow()
                .get(dd.selected() as usize)
                .copied()
                .flatten();
            *state.tint_filter.borrow_mut() = id;
            state.refresh_rows();
        });
    }
    {
        let state = Rc::clone(state);
        let shape_dropdown = state.shape_dropdown.clone();
        shape_dropdown.connect_selected_notify(move |dd| {
            let shape = state
                .shape_ids
                .borrow()
                .get(dd.selected() as usize)
                .copied()
                .flatten();
            *state.shape_filter.borrow_mut() = shape;
            state.refresh_rows();
        });
    }
    {
        let state = Rc::clone(state);
        let tag_dropdown = state.tag_dropdown.clone();
        tag_dropdown.connect_selected_notify(move |dd| {
            let id = state
                .tag_ids
                .borrow()
                .get(dd.selected() as usize)
                .copied()
                .flatten();
            *state.tag_filter.borrow_mut() = id;
            state.refresh_rows();
        });
    }
    {
        let state = Rc::clone(state);
        let find_entry = state.find_entry.clone();
        find_entry.connect_activate(move |_| apply_find_replace(&state));
    }
    {
        let state = Rc::clone(state);
        let replace_entry = state.replace_entry.clone();
        replace_entry.connect_activate(move |_| apply_find_replace(&state));
    }
}

fn apply_find_replace(state: &Rc<State>) {
    let find = state.find_entry.text().to_string();
    if find.is_empty() {
        return;
    }
    let replace = state.replace_entry.text().to_string();
    state.apply_to_visible(|item, _| item.name.replace(&find, &replace));
    state.refresh_rows();
}

fn rebuild_tint_dropdown(state: &Rc<State>, tints: &[TintRecord]) {
    let mut ids: Vec<Option<i64>> = vec![None];
    let mut names: Vec<String> = vec!["Any".to_string()];
    for tint in tints {
        ids.push(Some(tint.id));
        names.push(tint.name.clone());
    }
    *state.tint_ids.borrow_mut() = ids;
    *state.tint_filter.borrow_mut() = None;
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let list = StringList::new(&name_refs);
    state.tint_dropdown.set_model(Some(&list));
    state.tint_dropdown.set_selected(0);
}

fn rebuild_shape_dropdown(state: &Rc<State>) {
    let shapes = all_shapes();
    let mut ids: Vec<Option<Shape>> = vec![None];
    let mut names: Vec<String> = vec!["Any Shape".to_string()];
    for shape in shapes {
        ids.push(Some(shape));
        names.push(format!("{} {}", shape.glyph(), shape.display_name()));
    }
    *state.shape_ids.borrow_mut() = ids;
    *state.shape_filter.borrow_mut() = None;
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let list = StringList::new(&name_refs);
    state.shape_dropdown.set_model(Some(&list));
    state.shape_dropdown.set_selected(0);
}

fn rebuild_tag_dropdown(state: &Rc<State>, tags: &[TagRecord]) {
    let mut ids: Vec<Option<i64>> = vec![None];
    let mut names: Vec<String> = vec!["Any Tag".to_string()];
    for tag in tags {
        ids.push(Some(tag.id));
        names.push(format!("#{}", tag.name));
    }
    *state.tag_ids.borrow_mut() = ids;
    *state.tag_filter.borrow_mut() = None;
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let list = StringList::new(&name_refs);
    state.tag_dropdown.set_model(Some(&list));
    state.tag_dropdown.set_selected(0);
}

fn all_shapes() -> [Shape; 7] {
    [
        Shape::Circle,
        Shape::Square,
        Shape::Triangle,
        Shape::Pentagon,
        Shape::Hexagon,
        Shape::Octagon,
        Shape::Trapezoid,
    ]
}

fn recipe_entry(placeholder: &str) -> Entry {
    let entry = Entry::new();
    entry.add_css_class("bn-recipe-entry");
    entry.set_placeholder_text(Some(placeholder));
    entry.set_hexpand(true);
    entry
}

fn recipe_button(label: &str) -> Button {
    let button = Button::with_label(label);
    button.add_css_class("bn-recipe-button");
    button
}

fn ops_chip(label: &str) -> Button {
    let button = Button::with_label(label);
    button.add_css_class("bn-ops-chip");
    button
}

fn invalid_name_reason(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("Empty name")
    } else if name.contains('/') || name.contains('\0') {
        Some("Invalid character")
    } else {
        None
    }
}

fn append_suffix(name: &str, suffix: &str, is_dir: bool) -> String {
    if is_dir {
        return format!("{name}{suffix}");
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    if ext.is_empty() {
        format!("{stem}{suffix}")
    } else {
        format!("{stem}{suffix}.{ext}")
    }
}

fn numbered_name(item: &FileItem, number: usize) -> String {
    if item.is_dir {
        return format!("{number:03}-{}", item.name);
    }
    let path = Path::new(&item.name);
    let stem = path
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or(&item.name);
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    if ext.is_empty() {
        format!("{number:03}-{stem}")
    } else {
        format!("{number:03}-{stem}.{ext}")
    }
}

fn clean_case_name(name: &str, is_dir: bool) -> String {
    let path = Path::new(name);
    let (stem, ext) = if is_dir {
        (name, "")
    } else {
        (
            path.file_stem().and_then(|v| v.to_str()).unwrap_or(name),
            path.extension().and_then(|v| v.to_str()).unwrap_or(""),
        )
    };
    let cleaned = stem
        .replace(['_', '-'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if ext.is_empty() {
        cleaned
    } else {
        format!("{cleaned}.{ext}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_preserves_file_extension() {
        assert_eq!(
            append_suffix("photo.jpg", "-final", false),
            "photo-final.jpg"
        );
        assert_eq!(append_suffix("Folder", "-final", true), "Folder-final");
    }

    #[test]
    fn invalid_names_reject_empty_and_path_separators() {
        assert_eq!(invalid_name_reason(""), Some("Empty name"));
        assert_eq!(invalid_name_reason("a/b"), Some("Invalid character"));
        assert_eq!(invalid_name_reason("valid"), None);
    }

    #[test]
    fn clean_case_keeps_extension() {
        assert_eq!(
            clean_case_name("my_file-name.txt", false),
            "My File Name.txt"
        );
    }
}
