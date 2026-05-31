use crate::terroir_client::{
    TerroirBrokenRef, TerroirDoctorSummary, TerroirPalette, TerroirStatus, TerroirWorkspaceEntry,
};
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Button, Label, Orientation, ScrolledWindow, Separator};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatercolorPanelView {
    Status,
    Workspaces,
    Palettes,
    BrokenRefs,
}

#[derive(Clone, Debug)]
pub struct WatercolorPanelData {
    pub status: Result<TerroirStatus, String>,
    pub workspaces: Vec<TerroirWorkspaceEntry>,
    pub palettes: Vec<TerroirPalette>,
    pub broken_refs: Vec<TerroirBrokenRef>,
    pub doctor: Option<TerroirDoctorSummary>,
}

#[derive(Clone)]
pub struct WatercolorPanel {
    pub root: GtkBox,
    inner: GtkBox,
}

impl WatercolorPanel {
    pub fn build() -> Self {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("project-landing");
        root.set_vexpand(true);
        root.set_hexpand(true);
        root.set_visible(false);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();

        let inner = GtkBox::new(Orientation::Vertical, 10);
        inner.set_margin_start(12);
        inner.set_margin_end(12);
        inner.set_margin_top(12);
        inner.set_margin_bottom(12);
        scroll.set_child(Some(&inner));
        root.append(&scroll);

        Self { root, inner }
    }

    pub fn set_loading(&self, view: &WatercolorPanelView) {
        self.clear();
        self.inner.append(&heading(view_title(view)));
        self.inner.append(&note("Loading Watercolor context..."));
        self.root.set_visible(true);
    }

    pub fn populate<FOpen, FRefresh>(
        &self,
        view: WatercolorPanelView,
        data: &WatercolorPanelData,
        on_open_path: FOpen,
        on_refresh: FRefresh,
    ) where
        FOpen: Fn(PathBuf) + Clone + 'static,
        FRefresh: Fn() + Clone + 'static,
    {
        self.clear();
        self.inner.append(&heading(view_title(&view)));
        self.inner.append(&refresh_row(on_refresh));

        match view {
            WatercolorPanelView::Status => self.populate_status(data),
            WatercolorPanelView::Workspaces => self.populate_workspaces(data, on_open_path),
            WatercolorPanelView::Palettes => self.populate_palettes(data),
            WatercolorPanelView::BrokenRefs => self.populate_broken_refs(data, on_open_path),
        }

        self.root.set_visible(true);
    }

    fn populate_status(&self, data: &WatercolorPanelData) {
        match &data.status {
            Ok(status) => {
                self.inner.append(&kv("Terroir", "Running"));
                self.inner.append(&kv(
                    "Known Workspaces",
                    &status
                        .workspaces
                        .unwrap_or(data.workspaces.len())
                        .to_string(),
                ));
            }
            Err(error) => {
                self.inner.append(&kv("Terroir", "Unavailable"));
                self.inner.append(&note(error));
            }
        }

        if let Some(doctor) = &data.doctor {
            self.inner.append(&kv(
                "Index Health",
                &format!(
                    "{} warning, {} broken, {} dangerous",
                    doctor.counts.warning, doctor.counts.broken, doctor.counts.dangerous
                ),
            ));
        } else if data.workspaces.is_empty() {
            self.inner.append(&note(
                "No indexed Watercolor context yet. Start Terroir and refresh when .water workspaces are available.",
            ));
        }
    }

    fn populate_workspaces<FOpen>(&self, data: &WatercolorPanelData, on_open_path: FOpen)
    where
        FOpen: Fn(PathBuf) + Clone + 'static,
    {
        let visible: Vec<&TerroirWorkspaceEntry> = data
            .workspaces
            .iter()
            .filter(|workspace| {
                matches!(
                    workspace.state.as_str(),
                    "known" | "active" | "pinned" | "discovered"
                )
            })
            .collect();

        if visible.is_empty() {
            self.inner
                .append(&note("No active or known .water workspaces are indexed."));
            return;
        }

        for workspace in visible {
            let button = row_button(
                "Workspace",
                &workspace.path,
                &format!("{} · {}", workspace.state, workspace.water_file),
            );
            let path = PathBuf::from(&workspace.path);
            let on_open_path = on_open_path.clone();
            button.connect_clicked(move |_| on_open_path(path.clone()));
            self.inner.append(&button);
        }
    }

    fn populate_palettes(&self, data: &WatercolorPanelData) {
        if data.palettes.is_empty() {
            self.inner.append(&note(
                "No Watercolor palettes are indexed. Refresh after Terroir has discovered and indexed .water files.",
            ));
            return;
        }

        for palette in &data.palettes {
            let workspace = palette.workspace_name.as_deref().unwrap_or("Watercolor");
            self.inner.append(&kv(
                &format!("Watercolor Palette · {workspace}"),
                &format!("{} ({})", palette.palette_name, palette.palette_id),
            ));
        }

        self.inner.append(&note(
            "Terroir v0.1 exposes palette names here. Referenced files appear through selected-file context and broken-reference views.",
        ));
    }

    fn populate_broken_refs<FOpen>(&self, data: &WatercolorPanelData, on_open_path: FOpen)
    where
        FOpen: Fn(PathBuf) + Clone + 'static,
    {
        if data.broken_refs.is_empty() {
            self.inner
                .append(&note("No broken Watercolor file references."));
            return;
        }

        for file_ref in &data.broken_refs {
            let button = row_button(
                "Missing Reference",
                &file_ref.target_path,
                &format!("{} · {}", file_ref.workspace_name, file_ref.object_title),
            );
            button.set_sensitive(PathBuf::from(&file_ref.resolved_path).exists());
            let path = PathBuf::from(&file_ref.resolved_path);
            let on_open_path = on_open_path.clone();
            button.connect_clicked(move |_| on_open_path(path.clone()));
            self.inner.append(&button);
        }
    }

    fn clear(&self) {
        while let Some(child) = self.inner.first_child() {
            self.inner.remove(&child);
        }
    }
}

fn view_title(view: &WatercolorPanelView) -> &'static str {
    match view {
        WatercolorPanelView::Status => "Watercolor Context",
        WatercolorPanelView::Workspaces => "Watercolor Workspaces",
        WatercolorPanelView::Palettes => "Watercolor Palettes",
        WatercolorPanelView::BrokenRefs => "Broken Watercolor References",
    }
}

fn heading(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("landing-section-heading");
    label.set_halign(Align::Start);
    label
}

fn note(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("landing-dest-empty");
    label.set_halign(Align::Start);
    label.set_wrap(true);
    label
}

fn kv(label: &str, value: &str) -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 4);
    row.add_css_class("preview-meta");
    let key = Label::new(Some(label));
    key.add_css_class("preview-meta-key");
    key.set_halign(Align::Start);
    let val = Label::new(Some(value));
    val.add_css_class("preview-meta-value");
    val.set_halign(Align::Start);
    val.set_wrap(true);
    row.append(&key);
    row.append(&val);
    row
}

fn row_button(title: &str, main: &str, detail: &str) -> Button {
    let button = Button::new();
    button.add_css_class("context-menu-button");
    button.set_halign(Align::Fill);
    let outer = GtkBox::new(Orientation::Vertical, 4);
    outer.set_margin_top(6);
    outer.set_margin_bottom(6);
    outer.set_margin_start(8);
    outer.set_margin_end(8);
    let title = Label::new(Some(title));
    title.add_css_class("preview-meta-key");
    title.set_halign(Align::Start);
    let main = Label::new(Some(main));
    main.add_css_class("preview-meta-value");
    main.set_halign(Align::Start);
    main.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let detail = Label::new(Some(detail));
    detail.add_css_class("landing-dest-empty");
    detail.set_halign(Align::Start);
    detail.set_wrap(true);
    outer.append(&title);
    outer.append(&main);
    outer.append(&detail);
    button.set_child(Some(&outer));
    button
}

fn refresh_row<FRefresh>(on_refresh: FRefresh) -> GtkBox
where
    FRefresh: Fn() + 'static,
{
    let row = GtkBox::new(Orientation::Horizontal, 8);
    let button = Button::with_label("Refresh Watercolor Context");
    button.add_css_class("landing-add-btn");
    button.connect_clicked(move |_| on_refresh());
    row.append(&button);
    row.append(&Separator::new(Orientation::Horizontal));
    row
}
