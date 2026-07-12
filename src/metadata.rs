use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

const DB_FILE_NAME: &str = "metadata.db";
const DB_SCHEMA_VERSION: i32 = 9;
const DEFAULT_TINT_NAME: &str = "Beige";
const DEFAULT_TINT_COLOR: &str = "#806040";

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct TintRecord {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub position: i64,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Shape {
    Circle,
    Square,
    Triangle,
    Pentagon,
    Hexagon,
    Octagon,
    Trapezoid,
}

impl Shape {
    pub const DEFAULT: Self = Self::Square;

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Square => "square",
            Self::Triangle => "triangle",
            Self::Pentagon => "pentagon",
            Self::Hexagon => "hexagon",
            Self::Octagon => "octagon",
            Self::Trapezoid => "trapezoid",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Circle => "Circle",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Pentagon => "Pentagon",
            Self::Hexagon => "Hexagon",
            Self::Octagon => "Octagon",
            Self::Trapezoid => "Trapezoid",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Circle => "●",
            Self::Square => "■",
            Self::Triangle => "▲",
            Self::Pentagon => "⬠",
            Self::Hexagon => "⬡",
            Self::Octagon => "⯁",
            Self::Trapezoid => "⏢",
        }
    }
}

impl FromStr for Shape {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "circle" => Ok(Self::Circle),
            "square" => Ok(Self::Square),
            "triangle" => Ok(Self::Triangle),
            "pentagon" => Ok(Self::Pentagon),
            "hexagon" => Ok(Self::Hexagon),
            "octagon" => Ok(Self::Octagon),
            "trapezoid" => Ok(Self::Trapezoid),
            _ => Err(format!("Unsupported shape: {value}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct FileMarkRecord {
    pub file_path: PathBuf,
    pub tint_id: i64,
    pub shape: Shape,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRecord {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PalettePlaceRecord {
    pub id: i64,
    pub palette_id: i64,
    pub name: String,
    pub path: String,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct PaletteItemRecord {
    pub id: i64,
    pub palette_id: i64,
    pub item_type: String,
    pub path: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub tint_id: Option<i64>,
    pub shape: Option<Shape>,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct PaletteLinkRecord {
    pub id: i64,
    pub palette_id: i64,
    pub source_item_id: i64,
    pub target_item_id: i64,
    pub strength: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: i64,
    pub name: String,
    #[allow(dead_code)]
    pub color: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDestinationRecord {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceRecord {
    pub id: i64,
    pub name: String,
    pub folder_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudRecord {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub kind: String,
    /// Provider-specific remote identifier — for rclone this is the remote name (e.g. "gdrive").
    /// Used by Lattice to call `rclone mount <remote_name>:` without reading credentials.
    pub remote_name: Option<String>,
    pub notes: Option<String>,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagRecord {
    pub id: i64,
    pub name: String,
    #[allow(dead_code)]
    pub color: Option<String>,
    pub associated_tint_id: Option<i64>,
    pub associated_shape: Option<Shape>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ActivityLogEntry {
    pub id: i64,
    pub timestamp_ms: i64,
    pub operation: String,
    pub file_count: i32,
    pub summary: String,
    pub source_path: String,
    pub destination_path: Option<String>,
    pub status: String,
    pub error_detail: Option<String>,
    pub items: Vec<ActivityLogItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityLogItem {
    pub source_path: String,
    pub destination_path: Option<String>,
    pub status: String,
    pub error_detail: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FolderViewState {
    /// "icons" | "list"
    pub view_mode: String,
    pub show_hidden: bool,
    pub show_shape_badges: bool,
    /// "name" | "modified" | "size" | "kind"
    pub sort_field: String,
    /// "ascending" | "descending"
    pub sort_direction: String,
}

pub struct MetadataStore {
    conn: Connection,
}

#[allow(dead_code)]
impl MetadataStore {
    pub fn open() -> Result<Self, String> {
        let db_path = metadata_db_path();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create metadata directory: {error}"))?;
        }

        let conn = Connection::open(&db_path)
            .map_err(|error| format!("Failed to open metadata database: {error}"))?;
        backup_existing_metadata_if_needed(&conn, &db_path)?;
        let mut store = Self { conn };
        store.initialize()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|error| format!("Failed to open in-memory metadata database: {error}"))?;
        let mut store = Self { conn };
        store.initialize()?;
        Ok(store)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRecord>, String> {
        self.list_palettes().map(|palettes| {
            palettes
                .into_iter()
                .map(|palette| ProjectRecord {
                    id: palette.id,
                    name: palette.name,
                    color: None,
                })
                .collect()
        })
    }

    pub fn create_project(
        &mut self,
        name: &str,
        _color: Option<&str>,
    ) -> Result<ProjectRecord, String> {
        let palette = self.create_palette(name, None)?;
        Ok(ProjectRecord {
            id: palette.id,
            name: palette.name,
            color: None,
        })
    }

    pub fn rename_project(&mut self, id: i64, name: &str) -> Result<(), String> {
        self.rename_palette(id, name)
    }

    pub fn update_project_color(&mut self, _id: i64, _color: Option<&str>) -> Result<(), String> {
        // Projects are now Palettes. Palettes do not own visual color.
        Ok(())
    }

    pub fn list_project_destinations(
        &self,
        project_id: i64,
    ) -> Result<Vec<ProjectDestinationRecord>, String> {
        self.list_palette_places(project_id).map(|places| {
            places
                .into_iter()
                .map(|place| ProjectDestinationRecord {
                    id: place.id,
                    project_id: place.palette_id,
                    name: place.name,
                    path: place.path,
                })
                .collect()
        })
    }

    pub fn delete_project(&mut self, project_id: i64) -> Result<(), String> {
        self.delete_palette(project_id)
    }

    pub fn add_project_destination(
        &mut self,
        project_id: i64,
        name: &str,
        path: &str,
    ) -> Result<ProjectDestinationRecord, String> {
        let place = self.add_palette_place(project_id, name, path)?;
        Ok(ProjectDestinationRecord {
            id: place.id,
            project_id: place.palette_id,
            name: place.name,
            path: place.path,
        })
    }

    pub fn remove_project_destination(&mut self, destination_id: i64) -> Result<(), String> {
        self.remove_palette_place(destination_id)
    }

    pub fn list_tints(&self) -> Result<Vec<TintRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, color, position, is_default
                 FROM tints
                 ORDER BY position ASC, name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], tint_from_row)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn create_tint(&mut self, name: &str, color: &str) -> Result<TintRecord, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Tint name cannot be empty.".to_string());
        }
        let next_position = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM tints",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        self.conn
            .execute(
                "INSERT INTO tints (name, color, position, is_default)
                 VALUES (?1, ?2, ?3, 0)",
                params![trimmed, color.trim(), next_position],
            )
            .map_err(map_constraint_error)?;
        let id = self.conn.last_insert_rowid();
        self.tint_by_id(id)?
            .ok_or_else(|| "Failed to reload saved tint.".to_string())
    }

    pub fn rename_tint(&mut self, id: i64, name: &str) -> Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Tint name cannot be empty.".to_string());
        }
        self.conn
            .execute(
                "UPDATE tints SET name = ?1 WHERE id = ?2",
                params![trimmed, id],
            )
            .map_err(map_constraint_error)?;
        Ok(())
    }

    pub fn update_tint_color(&mut self, id: i64, color: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE tints SET color = ?1 WHERE id = ?2",
                params![color.trim(), id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_tint(&mut self, id: i64) -> Result<(), String> {
        let tint = self
            .tint_by_id(id)?
            .ok_or_else(|| format!("Tint {id} not found."))?;
        if tint.is_default {
            return Err("The default Tint cannot be deleted.".to_string());
        }
        let usage_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM file_marks WHERE tint_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if usage_count > 0 {
            return Err("Tint is used by file marks and cannot be deleted.".to_string());
        }
        let tag_usage_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tags WHERE associated_tint_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if tag_usage_count > 0 {
            return Err("Tint is associated with tags and cannot be deleted.".to_string());
        }
        self.conn
            .execute("DELETE FROM tints WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn mark_for_path(&self, path: &Path) -> Result<FileMarkRecord, String> {
        let normalized = normalize_path(path);
        self.explicit_mark_for_path(&normalized)?
            .map(Ok)
            .unwrap_or_else(|| self.default_file_mark(PathBuf::from(normalized)))
    }

    pub fn marks_for_paths(
        &self,
        paths: &[PathBuf],
    ) -> Result<HashMap<PathBuf, FileMarkRecord>, String> {
        let mut marks = HashMap::new();
        if paths.is_empty() {
            return Ok(marks);
        }

        let normalized = paths
            .iter()
            .map(|path| normalize_path(path))
            .collect::<Vec<_>>();
        let placeholders = repeat_vars(normalized.len());
        let sql = format!(
            "SELECT file_path, tint_id, shape, created_at, updated_at
             FROM file_marks
             WHERE file_path IN ({placeholders})"
        );
        let mut statement = self.conn.prepare(&sql).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params_from_iter(normalized.iter()), file_mark_from_row)
            .map_err(|error| error.to_string())?;

        for row in rows {
            let mark = row.map_err(|error| error.to_string())?;
            marks.insert(mark.file_path.clone(), mark);
        }

        for path in paths {
            let normalized_path = PathBuf::from(normalize_path(path));
            if !marks.contains_key(&normalized_path) {
                marks.insert(
                    normalized_path.clone(),
                    self.default_file_mark(normalized_path)?,
                );
            }
        }

        Ok(marks)
    }

    pub fn set_file_mark(
        &mut self,
        path: &Path,
        tint_id: i64,
        shape: Shape,
    ) -> Result<FileMarkRecord, String> {
        if self.tint_by_id(tint_id)?.is_none() {
            return Err(format!("Tint {tint_id} not found."));
        }
        let normalized = normalize_path(path);
        let now = now_ms();
        self.conn
            .execute(
                "INSERT INTO file_marks (file_path, tint_id, shape, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(file_path) DO UPDATE SET
                    tint_id = excluded.tint_id,
                    shape = excluded.shape,
                    updated_at = excluded.updated_at",
                params![normalized, tint_id, shape.as_str(), now],
            )
            .map_err(|error| error.to_string())?;
        self.mark_for_path(path)
    }

    pub fn clear_file_mark(&mut self, path: &Path) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM file_marks WHERE file_path = ?1",
                params![normalize_path(path)],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn list_palettes(&self) -> Result<Vec<PaletteRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, description, created_at, updated_at
                 FROM palettes
                 ORDER BY name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], palette_from_row)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn create_palette(
        &mut self,
        name: &str,
        description: Option<&str>,
    ) -> Result<PaletteRecord, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Palette name cannot be empty.".to_string());
        }
        let now = now_ms();
        self.conn
            .execute(
                "INSERT INTO palettes (name, description, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)",
                params![trimmed, description.map(str::trim), now],
            )
            .map_err(map_constraint_error)?;
        let id = self.conn.last_insert_rowid();
        self.palette_by_id(id)?
            .ok_or_else(|| "Failed to reload saved palette.".to_string())
    }

    pub fn rename_palette(&mut self, id: i64, name: &str) -> Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Palette name cannot be empty.".to_string());
        }
        self.conn
            .execute(
                "UPDATE palettes SET name = ?1, updated_at = ?2 WHERE id = ?3",
                params![trimmed, now_ms(), id],
            )
            .map_err(map_constraint_error)?;
        Ok(())
    }

    pub fn delete_palette(&mut self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM palettes WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn list_palette_places(&self, palette_id: i64) -> Result<Vec<PalettePlaceRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, palette_id, name, path, position
                 FROM palette_places
                 WHERE palette_id = ?1
                 ORDER BY position ASC, id ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![palette_id], palette_place_from_row)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn add_palette_place(
        &mut self,
        palette_id: i64,
        name: &str,
        path: &str,
    ) -> Result<PalettePlaceRecord, String> {
        let trimmed_name = name.trim();
        let trimmed_path = path.trim();
        if trimmed_name.is_empty() {
            return Err("Palette place name cannot be empty.".to_string());
        }
        if trimmed_path.is_empty() {
            return Err("Palette place path cannot be empty.".to_string());
        }
        let next_position = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), 0) + 1
                 FROM palette_places
                 WHERE palette_id = ?1",
                params![palette_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        self.conn
            .execute(
                "INSERT INTO palette_places (palette_id, name, path, position)
                 VALUES (?1, ?2, ?3, ?4)",
                params![palette_id, trimmed_name, trimmed_path, next_position],
            )
            .map_err(map_constraint_error)?;
        Ok(PalettePlaceRecord {
            id: self.conn.last_insert_rowid(),
            palette_id,
            name: trimmed_name.to_string(),
            path: trimmed_path.to_string(),
            position: next_position,
        })
    }

    pub fn remove_palette_place(&mut self, place_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM palette_places WHERE id = ?1",
                params![place_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn list_project_activity(&self, root_path: &str, limit: usize) -> Vec<ActivityLogEntry> {
        let prefix = format!("{root_path}/%");
        let mut stmt = match self.conn.prepare(
            "SELECT id, timestamp_ms, operation, file_count, summary,
                    source_path, destination_path, status, error_detail
             FROM activity_log
             WHERE source_path = ?1 OR source_path LIKE ?2
                OR (destination_path IS NOT NULL
                    AND (destination_path = ?1 OR destination_path LIKE ?2))
             ORDER BY timestamp_ms DESC, id DESC
             LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut entries: Vec<ActivityLogEntry> = stmt
            .query_map(params![root_path, prefix, limit as i64], |row| {
                Ok(ActivityLogEntry {
                    id: row.get(0)?,
                    timestamp_ms: row.get(1)?,
                    operation: row.get(2)?,
                    file_count: row.get(3)?,
                    summary: row.get(4)?,
                    source_path: row.get(5)?,
                    destination_path: row.get(6)?,
                    status: row.get(7)?,
                    error_detail: row.get(8)?,
                    items: Vec::new(),
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        for entry in &mut entries {
            entry.items = self.list_activity_items(entry.id);
        }
        entries
    }

    pub fn list_places(&self) -> Result<Vec<PlaceRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, folder_path
                 FROM places
                 ORDER BY position ASC, name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(PlaceRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    folder_path: PathBuf::from(row.get::<_, String>(2)?),
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn create_place(&mut self, name: &str, folder_path: &Path) -> Result<PlaceRecord, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Place names cannot be empty.".to_string());
        }
        let normalized_path = normalize_path(folder_path);
        if normalized_path.is_empty() {
            return Err("Place folder path cannot be empty.".to_string());
        }
        let next_position = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM places",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        self.conn
            .execute(
                "INSERT INTO places (name, folder_path, position) VALUES (?1, ?2, ?3)
                 ON CONFLICT(folder_path) DO UPDATE SET name = excluded.name",
                params![trimmed, normalized_path, next_position],
            )
            .map_err(map_constraint_error)?;
        self.find_place_by_path(folder_path)?
            .ok_or_else(|| "Failed to reload saved place.".to_string())
    }

    pub fn remove_place(&mut self, place_id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM places WHERE id = ?1", params![place_id])
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn list_cloud_locations(&self) -> Result<Vec<CloudRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, path, kind, remote_name, notes, position
                 FROM cloud_locations
                 ORDER BY position ASC, name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(CloudRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    kind: row.get(3)?,
                    remote_name: row.get(4)?,
                    notes: row.get(5)?,
                    position: row.get(6)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn create_cloud_location(
        &mut self,
        name: &str,
        path: &str,
        kind: &str,
        remote_name: Option<&str>,
        notes: Option<&str>,
    ) -> Result<CloudRecord, String> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err("Cloud location name cannot be empty.".to_string());
        }
        let trimmed_path = path.trim();
        if trimmed_path.is_empty() {
            return Err("Cloud location path cannot be empty.".to_string());
        }
        let trimmed_remote = remote_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let next_position = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM cloud_locations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        self.conn
            .execute(
                "INSERT INTO cloud_locations (name, path, kind, remote_name, notes, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    trimmed_name,
                    trimmed_path,
                    kind,
                    trimmed_remote,
                    notes,
                    next_position
                ],
            )
            .map_err(map_constraint_error)?;
        let id = self.conn.last_insert_rowid();
        Ok(CloudRecord {
            id,
            name: trimmed_name.to_string(),
            path: trimmed_path.to_string(),
            kind: kind.to_string(),
            remote_name: trimmed_remote,
            notes: notes.map(|s| s.to_string()),
            position: next_position,
        })
    }

    pub fn update_cloud_location(
        &mut self,
        id: i64,
        name: &str,
        path: &str,
        kind: &str,
        remote_name: Option<&str>,
        notes: Option<&str>,
    ) -> Result<(), String> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err("Cloud location name cannot be empty.".to_string());
        }
        let trimmed_path = path.trim();
        if trimmed_path.is_empty() {
            return Err("Cloud location path cannot be empty.".to_string());
        }
        let trimmed_remote = remote_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self.conn
            .execute(
                "UPDATE cloud_locations
                 SET name = ?1, path = ?2, kind = ?3, remote_name = ?4, notes = ?5
                 WHERE id = ?6",
                params![trimmed_name, trimmed_path, kind, trimmed_remote, notes, id],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn delete_cloud_location(&mut self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM cloud_locations WHERE id = ?1", params![id])
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn list_tags(&self) -> Result<Vec<TagRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, color, associated_tint_id, associated_shape
                 FROM tags
                 ORDER BY name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], tag_from_row)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn ensure_tag(&mut self, name: &str) -> Result<TagRecord, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Tag names cannot be empty.".to_string());
        }

        if let Some(existing) = self.find_tag_by_name(trimmed)? {
            return Ok(existing);
        }

        self.conn
            .execute("INSERT INTO tags (name) VALUES (?1)", params![trimmed])
            .map_err(map_constraint_error)?;
        let id = self.conn.last_insert_rowid();
        self.find_tag_by_id(id)?
            .ok_or_else(|| "Failed to reload saved tag.".to_string())
    }

    pub fn add_tag_to_paths(&mut self, tag_id: i64, paths: &[PathBuf]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(|error| error.to_string())?;
        for path in paths {
            tx.execute(
                "INSERT OR IGNORE INTO file_tags (file_path, tag_id) VALUES (?1, ?2)",
                params![normalize_path(path), tag_id],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn remove_tag_from_paths(&mut self, tag_id: i64, paths: &[PathBuf]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(|error| error.to_string())?;
        for path in paths {
            tx.execute(
                "DELETE FROM file_tags WHERE file_path = ?1 AND tag_id = ?2",
                params![normalize_path(path), tag_id],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn move_tagged_path_prefix(
        &mut self,
        old_prefix: &Path,
        new_prefix: &Path,
    ) -> Result<(), String> {
        let old_prefix = normalize_path(old_prefix);
        let new_prefix = normalize_path(new_prefix);
        let matches = self.list_tagged_prefix_rows(&old_prefix)?;
        let mark_matches = self.list_marked_prefix_rows(&old_prefix)?;
        if matches.is_empty() && mark_matches.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction().map_err(|error| error.to_string())?;
        for (current_path, tag_id) in &matches {
            let remapped = remap_prefix_path(current_path, &old_prefix, &new_prefix)?;
            tx.execute(
                "INSERT OR IGNORE INTO file_tags (file_path, tag_id) VALUES (?1, ?2)",
                params![remapped, tag_id],
            )
            .map_err(|error| error.to_string())?;
        }

        tx.execute(
            "DELETE FROM file_tags WHERE file_path = ?1 OR file_path LIKE ?2",
            params![old_prefix, format!("{}/%", old_prefix)],
        )
        .map_err(|error| error.to_string())?;
        for (current_path, tint_id, shape, created_at, updated_at) in &mark_matches {
            let remapped = remap_prefix_path(current_path, &old_prefix, &new_prefix)?;
            tx.execute(
                "INSERT INTO file_marks (file_path, tint_id, shape, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(file_path) DO UPDATE SET
                    tint_id = excluded.tint_id,
                    shape = excluded.shape,
                    updated_at = excluded.updated_at",
                params![remapped, tint_id, shape, created_at, updated_at],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.execute(
            "DELETE FROM file_marks WHERE file_path = ?1 OR file_path LIKE ?2",
            params![old_prefix, format!("{}/%", old_prefix)],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn delete_tagged_path_prefix(&mut self, prefix: &Path) -> Result<(), String> {
        let prefix = normalize_path(prefix);
        self.conn
            .execute(
                "DELETE FROM file_tags WHERE file_path = ?1 OR file_path LIKE ?2",
                params![prefix, format!("{}/%", prefix)],
            )
            .map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "DELETE FROM file_marks WHERE file_path = ?1 OR file_path LIKE ?2",
                params![prefix, format!("{}/%", prefix)],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn tags_for_paths(
        &self,
        paths: &[PathBuf],
    ) -> Result<HashMap<PathBuf, Vec<TagRecord>>, String> {
        if paths.is_empty() {
            return Ok(HashMap::new());
        }

        let normalized = paths
            .iter()
            .map(|path| normalize_path(path))
            .collect::<Vec<_>>();
        let placeholders = repeat_vars(normalized.len());
        let sql = format!(
            "SELECT ft.file_path, t.id, t.name, t.color, t.associated_tint_id, t.associated_shape
             FROM file_tags ft
             JOIN tags t ON t.id = ft.tag_id
             WHERE ft.file_path IN ({placeholders})
             ORDER BY t.name COLLATE NOCASE ASC"
        );

        let mut statement = self.conn.prepare(&sql).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params_from_iter(normalized.iter()), |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    TagRecord {
                        id: row.get(1)?,
                        name: row.get(2)?,
                        color: row.get(3)?,
                        associated_tint_id: row.get(4)?,
                        associated_shape: shape_from_optional_lossy(row.get(5)?),
                    },
                ))
            })
            .map_err(|error| error.to_string())?;

        let mut tags_by_path = HashMap::new();
        for row in rows {
            let (path, tag) = row.map_err(|error| error.to_string())?;
            tags_by_path.entry(path).or_insert_with(Vec::new).push(tag);
        }
        Ok(tags_by_path)
    }

    pub fn tags_for_selection(&self, paths: &[PathBuf]) -> Result<Vec<TagRecord>, String> {
        let mut tags = self
            .tags_for_paths(paths)?
            .into_values()
            .flatten()
            .collect::<Vec<_>>();
        tags.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        tags.dedup_by(|left, right| left.id == right.id);
        Ok(tags)
    }

    pub fn rename_tag(&mut self, id: i64, new_name: &str) -> Result<(), String> {
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err("Tag name cannot be empty.".to_string());
        }
        let rows = self
            .conn
            .execute(
                "UPDATE tags SET name = ?1 WHERE id = ?2",
                params![trimmed, id],
            )
            .map_err(map_constraint_error)?;
        if rows == 0 {
            return Err(format!("Tag {id} not found."));
        }
        Ok(())
    }

    pub fn update_tag_color(&mut self, _id: i64, _color: &str) -> Result<(), String> {
        // Tag color is deprecated. Tags may suggest a Tint/Shape but do not own visual color.
        Ok(())
    }

    pub fn delete_tag(&mut self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM tags WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn count_files_per_tag(&self) -> Result<HashMap<i64, usize>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag_id, COUNT(*) FROM file_tags GROUP BY tag_id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, usize>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut map = HashMap::new();
        for row in rows {
            let (tag_id, count) = row.map_err(|e| e.to_string())?;
            map.insert(tag_id, count);
        }
        Ok(map)
    }

    pub fn list_paths_for_tag(&self, tag_id: i64) -> Result<Vec<PathBuf>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT file_path
                 FROM file_tags
                 WHERE tag_id = ?1
                 ORDER BY file_path COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![tag_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;

        rows.map(|row| row.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn record_recent_location(&mut self, path: &Path) -> Result<(), String> {
        let normalized_path = normalize_path(path);
        if normalized_path.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction().map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM recent_locations WHERE folder_path = ?1",
            params![normalized_path],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO recent_locations (folder_path, last_visited_unix)
             VALUES (?1, CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER))",
            params![normalized_path],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM recent_locations
             WHERE id NOT IN (
                 SELECT id FROM recent_locations
                 ORDER BY last_visited_unix DESC, id DESC
                 LIMIT 50
             )",
            [],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn list_recent_locations(&self, limit: usize) -> Result<Vec<PathBuf>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT folder_path
                 FROM recent_locations
                 ORDER BY last_visited_unix DESC, id DESC
                 LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;

        rows.map(|row| row.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn remove_recent_locations(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction().map_err(|error| error.to_string())?;
        for path in paths {
            tx.execute(
                "DELETE FROM recent_locations WHERE folder_path = ?1",
                params![normalize_path(path)],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn log_activity(
        &self,
        operation: &str,
        file_count: i32,
        summary: &str,
        source_path: &str,
        destination_path: Option<&str>,
        errors: &[String],
    ) -> Result<i64, String> {
        self.log_activity_with_items(
            operation,
            file_count,
            summary,
            source_path,
            destination_path,
            errors,
            &[],
        )
    }

    pub fn log_activity_with_items(
        &self,
        operation: &str,
        file_count: i32,
        summary: &str,
        source_path: &str,
        destination_path: Option<&str>,
        errors: &[String],
        items: &[(PathBuf, Option<PathBuf>)],
    ) -> Result<i64, String> {
        let status = if errors.is_empty() {
            "success"
        } else {
            "failed"
        };
        let error_detail: Option<&str> = errors.first().map(|s| s.as_str());
        let now_ms: i64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.conn
            .execute(
                "INSERT INTO activity_log
                 (timestamp_ms, operation, file_count, summary, source_path,
                  destination_path, status, error_detail)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    now_ms,
                    operation,
                    file_count,
                    summary,
                    source_path,
                    destination_path,
                    status,
                    error_detail
                ],
            )
            .map_err(|e| e.to_string())?;
        let activity_id = self.conn.last_insert_rowid();
        for (index, (source, destination)) in items.iter().enumerate() {
            self.conn
                .execute(
                    "INSERT INTO activity_log_items
                     (activity_id, item_index, source_path, destination_path, status, error_detail)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                    params![
                        activity_id,
                        index as i64,
                        normalize_path(source),
                        destination.as_ref().map(|path| normalize_path(path)),
                        status
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(activity_id)
    }

    pub fn list_recent_activity(&self, limit: usize) -> Vec<ActivityLogEntry> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, timestamp_ms, operation, file_count, summary,
                    source_path, destination_path, status, error_detail
             FROM activity_log
             ORDER BY timestamp_ms DESC, id DESC
             LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut entries: Vec<ActivityLogEntry> = stmt
            .query_map(params![limit as i64], |row| {
                Ok(ActivityLogEntry {
                    id: row.get(0)?,
                    timestamp_ms: row.get(1)?,
                    operation: row.get(2)?,
                    file_count: row.get(3)?,
                    summary: row.get(4)?,
                    source_path: row.get(5)?,
                    destination_path: row.get(6)?,
                    status: row.get(7)?,
                    error_detail: row.get(8)?,
                    items: Vec::new(),
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        for entry in &mut entries {
            entry.items = self.list_activity_items(entry.id);
        }
        entries
    }

    pub fn delete_activity_before(&self, cutoff_ms: i64) -> usize {
        self.conn
            .execute(
                "DELETE FROM activity_log WHERE timestamp_ms < ?1",
                params![cutoff_ms],
            )
            .unwrap_or(0)
    }

    pub fn get_folder_view_state(&self, folder_path: &str) -> Option<FolderViewState> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT view_mode, show_hidden, show_shape_badges, sort_field, sort_direction
                 FROM folder_view_state WHERE folder_path = ?1",
            )
            .ok()?;
        stmt.query_row(params![folder_path], |row| {
            Ok(FolderViewState {
                view_mode: row.get(0)?,
                show_hidden: row.get::<_, i64>(1)? != 0,
                show_shape_badges: row.get::<_, i64>(2)? != 0,
                sort_field: row.get(3)?,
                sort_direction: row.get(4)?,
            })
        })
        .ok()
    }

    pub fn set_folder_view_state(&self, folder_path: &str, state: &FolderViewState) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO folder_view_state
             (folder_path, view_mode, show_hidden, show_shape_badges, sort_field, sort_direction, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                folder_path,
                state.view_mode,
                state.show_hidden as i64,
                state.show_shape_badges as i64,
                state.sort_field,
                state.sort_direction,
                now
            ],
        );
    }

    fn list_activity_items(&self, activity_id: i64) -> Vec<ActivityLogItem> {
        let mut stmt = match self.conn.prepare(
            "SELECT source_path, destination_path, status, error_detail
             FROM activity_log_items
             WHERE activity_id = ?1
             ORDER BY item_index ASC, id ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![activity_id], |row| {
            Ok(ActivityLogItem {
                source_path: row.get(0)?,
                destination_path: row.get(1)?,
                status: row.get(2)?,
                error_detail: row.get(3)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    fn initialize(&mut self) -> Result<(), String> {
        self.conn
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|error| error.to_string())?;

        let _version = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .map_err(|error| error.to_string())?;
        self.conn
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;

                CREATE TABLE IF NOT EXISTS projects (
                    id    INTEGER PRIMARY KEY AUTOINCREMENT,
                    name  TEXT NOT NULL UNIQUE,
                    color TEXT
                );

                CREATE TABLE IF NOT EXISTS tags (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    color TEXT,
                    associated_tint_id INTEGER REFERENCES tints(id) ON DELETE SET NULL,
                    associated_shape TEXT
                );

                CREATE TABLE IF NOT EXISTS file_tags (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    file_path TEXT NOT NULL,
                    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
                    UNIQUE(file_path, tag_id)
                );

                CREATE TABLE IF NOT EXISTS project_destinations (
                    id         INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    name       TEXT NOT NULL,
                    path       TEXT NOT NULL DEFAULT ''
                );

                CREATE TABLE IF NOT EXISTS tints (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    color TEXT NOT NULL,
                    position INTEGER NOT NULL DEFAULT 0,
                    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1))
                );

                CREATE TABLE IF NOT EXISTS file_marks (
                    file_path TEXT PRIMARY KEY,
                    tint_id INTEGER NOT NULL REFERENCES tints(id) ON DELETE RESTRICT,
                    shape TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS palettes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    description TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS palette_places (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    palette_id INTEGER NOT NULL REFERENCES palettes(id) ON DELETE CASCADE,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    position INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS palette_items (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    palette_id INTEGER NOT NULL REFERENCES palettes(id) ON DELETE CASCADE,
                    item_type TEXT NOT NULL CHECK (item_type IN ('file', 'folder', 'note')),
                    path TEXT,
                    title TEXT,
                    body TEXT,
                    tint_id INTEGER REFERENCES tints(id) ON DELETE SET NULL,
                    shape TEXT,
                    x INTEGER NOT NULL DEFAULT 0,
                    y INTEGER NOT NULL DEFAULT 0,
                    width INTEGER NOT NULL DEFAULT 220,
                    height INTEGER NOT NULL DEFAULT 160,
                    position INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS palette_links (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    palette_id INTEGER NOT NULL REFERENCES palettes(id) ON DELETE CASCADE,
                    source_item_id INTEGER NOT NULL REFERENCES palette_items(id) ON DELETE CASCADE,
                    target_item_id INTEGER NOT NULL REFERENCES palette_items(id) ON DELETE CASCADE,
                    strength TEXT NOT NULL CHECK (strength IN ('weak', 'strong')),
                    label TEXT
                );

                CREATE TABLE IF NOT EXISTS places (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    folder_path TEXT NOT NULL UNIQUE,
                    position INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS cloud_locations (
                    id       INTEGER PRIMARY KEY AUTOINCREMENT,
                    name     TEXT NOT NULL,
                    path     TEXT NOT NULL,
                    kind     TEXT NOT NULL DEFAULT 'manual',
                    notes    TEXT,
                    position INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS recent_locations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    folder_path TEXT NOT NULL UNIQUE,
                    last_visited_unix INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS activity_log (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    timestamp_ms INTEGER NOT NULL,
                    operation TEXT NOT NULL,
                    file_count INTEGER NOT NULL DEFAULT 1,
                    summary TEXT NOT NULL,
                    source_path TEXT NOT NULL,
                    destination_path TEXT,
                    status TEXT NOT NULL DEFAULT 'success',
                    error_detail TEXT
                );

                CREATE TABLE IF NOT EXISTS activity_log_items (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    activity_id INTEGER NOT NULL REFERENCES activity_log(id) ON DELETE CASCADE,
                    item_index INTEGER NOT NULL,
                    source_path TEXT NOT NULL,
                    destination_path TEXT,
                    status TEXT NOT NULL DEFAULT 'success',
                    error_detail TEXT
                );

                CREATE TABLE IF NOT EXISTS folder_view_state (
                    folder_path       TEXT PRIMARY KEY,
                    view_mode         TEXT NOT NULL DEFAULT 'icons',
                    show_hidden       INTEGER NOT NULL DEFAULT 0,
                    show_shape_badges INTEGER NOT NULL DEFAULT 1,
                    sort_field        TEXT NOT NULL DEFAULT 'name',
                    sort_direction    TEXT NOT NULL DEFAULT 'ascending',
                    updated_at        INTEGER NOT NULL
                );
                ",
            )
            .map_err(|error| error.to_string())?;

        self.ensure_column(
            "tags",
            "associated_tint_id",
            "INTEGER REFERENCES tints(id) ON DELETE SET NULL",
        )?;
        self.ensure_column("tags", "associated_shape", "TEXT")?;
        // Cloud Profiles: remote_name stores the provider-specific identifier (e.g. rclone remote name)
        self.ensure_column("cloud_locations", "remote_name", "TEXT")?;
        self.seed_default_tint()?;
        self.migrate_projects_to_palettes()?;

        self.conn
            .pragma_update(None, "user_version", DB_SCHEMA_VERSION)
            .map_err(|error| error.to_string())?;
        self.ensure_indexes()?;
        Ok(())
    }

    fn ensure_indexes(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "
                CREATE INDEX IF NOT EXISTS idx_file_tags_file_path ON file_tags(file_path);
                CREATE INDEX IF NOT EXISTS idx_file_tags_tag_id ON file_tags(tag_id);
                CREATE INDEX IF NOT EXISTS idx_project_destinations_project_id ON project_destinations(project_id);
                CREATE INDEX IF NOT EXISTS idx_file_marks_tint_id ON file_marks(tint_id);
                CREATE INDEX IF NOT EXISTS idx_palette_places_palette_id ON palette_places(palette_id);
                CREATE INDEX IF NOT EXISTS idx_palette_items_palette_id ON palette_items(palette_id);
                CREATE INDEX IF NOT EXISTS idx_palette_links_palette_id ON palette_links(palette_id);
                CREATE INDEX IF NOT EXISTS idx_palette_links_source ON palette_links(source_item_id);
                CREATE INDEX IF NOT EXISTS idx_palette_links_target ON palette_links(target_item_id);
                CREATE INDEX IF NOT EXISTS idx_recent_locations_last_visited ON recent_locations(last_visited_unix DESC);
                CREATE INDEX IF NOT EXISTS idx_activity_log_timestamp ON activity_log(timestamp_ms DESC);
                CREATE INDEX IF NOT EXISTS idx_activity_log_items_activity_id ON activity_log_items(activity_id, item_index, id);
                ",
            )
            .map_err(|error| error.to_string())
    }

    fn ensure_column(&self, table: &str, column: &str, definition: &str) -> Result<(), String> {
        let mut statement = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| error.to_string())?;
        for row in rows {
            if row.map_err(|error| error.to_string())? == column {
                return Ok(());
            }
        }
        self.conn
            .execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition};"
            ))
            .map_err(|error| error.to_string())
    }

    fn seed_default_tint(&self) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO tints (name, color, position, is_default)
                 VALUES (?1, ?2, 0, 1)
                 ON CONFLICT(name) DO UPDATE SET
                    is_default = CASE WHEN tints.is_default = 1 THEN 1 ELSE tints.is_default END",
                params![DEFAULT_TINT_NAME, DEFAULT_TINT_COLOR],
            )
            .map_err(|error| error.to_string())?;

        let default_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tints WHERE is_default = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if default_count == 0 {
            self.conn
                .execute(
                    "UPDATE tints SET is_default = 1 WHERE name = ?1",
                    params![DEFAULT_TINT_NAME],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn migrate_projects_to_palettes(&self) -> Result<(), String> {
        let now = now_ms();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO palettes (id, name, description, created_at, updated_at)
                 SELECT id, name, NULL, ?1, ?1 FROM projects",
                params![now],
            )
            .map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO palette_places (id, palette_id, name, path, position)
                 SELECT id, project_id, name, path, id FROM project_destinations",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn tint_by_id(&self, id: i64) -> Result<Option<TintRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, color, position, is_default FROM tints WHERE id = ?1",
                params![id],
                tint_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn default_tint(&self) -> Result<TintRecord, String> {
        self.conn
            .query_row(
                "SELECT id, name, color, position, is_default
                 FROM tints
                 WHERE is_default = 1
                 ORDER BY id ASC
                 LIMIT 1",
                [],
                tint_from_row,
            )
            .map_err(|error| error.to_string())
    }

    fn default_file_mark(&self, file_path: PathBuf) -> Result<FileMarkRecord, String> {
        let tint = self.default_tint()?;
        Ok(FileMarkRecord {
            file_path,
            tint_id: tint.id,
            shape: Shape::DEFAULT,
            created_at: 0,
            updated_at: 0,
        })
    }

    fn explicit_mark_for_path(
        &self,
        normalized_path: &str,
    ) -> Result<Option<FileMarkRecord>, String> {
        self.conn
            .query_row(
                "SELECT file_path, tint_id, shape, created_at, updated_at
                 FROM file_marks
                 WHERE file_path = ?1",
                params![normalized_path],
                file_mark_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn palette_by_id(&self, id: i64) -> Result<Option<PaletteRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, description, created_at, updated_at
                 FROM palettes
                 WHERE id = ?1",
                params![id],
                palette_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn project_by_id(&self, id: i64) -> Result<Option<ProjectRecord>, String> {
        Ok(self.palette_by_id(id)?.map(|palette| ProjectRecord {
            id: palette.id,
            name: palette.name,
            color: None,
        }))
    }

    fn find_place_by_path(&self, path: &Path) -> Result<Option<PlaceRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, folder_path FROM places WHERE folder_path = ?1",
                params![normalize_path(path)],
                |row| {
                    Ok(PlaceRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        folder_path: PathBuf::from(row.get::<_, String>(2)?),
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn find_tag_by_id(&self, id: i64) -> Result<Option<TagRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, color, associated_tint_id, associated_shape
                 FROM tags
                 WHERE id = ?1",
                params![id],
                tag_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn find_tag_by_name(&self, name: &str) -> Result<Option<TagRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, color, associated_tint_id, associated_shape
                 FROM tags
                 WHERE lower(name) = lower(?1)",
                params![name],
                tag_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn list_tagged_prefix_rows(&self, prefix: &str) -> Result<Vec<(String, i64)>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT file_path, tag_id
                 FROM file_tags
                 WHERE file_path = ?1 OR file_path LIKE ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![prefix, format!("{}/%", prefix)], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn list_marks_under_prefix(
        &self,
        prefix: &Path,
    ) -> Result<Vec<(PathBuf, i64, Shape)>, String> {
        let prefix_str = normalize_path(prefix);
        let rows = self.list_marked_prefix_rows(&prefix_str)?;
        Ok(rows
            .into_iter()
            .map(|(path, tint_id, shape_str, _, _)| {
                (
                    PathBuf::from(path),
                    tint_id,
                    shape_str.parse().unwrap_or(Shape::DEFAULT),
                )
            })
            .collect())
    }

    fn list_marked_prefix_rows(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, i64, String, i64, i64)>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT file_path, tint_id, shape, created_at, updated_at
                 FROM file_marks
                 WHERE file_path = ?1 OR file_path LIKE ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![prefix, format!("{}/%", prefix)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    // ── Palette Items ──────────────────────────────────────────────────────────

    pub fn list_palette_items(&self, palette_id: i64) -> Result<Vec<PaletteItemRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, palette_id, item_type, path, title, body,
                        tint_id, shape, x, y, width, height, position
                 FROM palette_items
                 WHERE palette_id = ?1
                 ORDER BY position ASC, id ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![palette_id], palette_item_from_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_palette_item(
        &mut self,
        palette_id: i64,
        item_type: &str,
        path: Option<&str>,
        title: Option<&str>,
        body: Option<&str>,
        tint_id: Option<i64>,
        shape: Option<Shape>,
        x: i64,
        y: i64,
        w: i64,
        h: i64,
    ) -> Result<PaletteItemRecord, String> {
        let next_position = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM palette_items WHERE palette_id = ?1",
                params![palette_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        self.conn
            .execute(
                "INSERT INTO palette_items
                 (palette_id, item_type, path, title, body, tint_id, shape,
                  x, y, width, height, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    palette_id,
                    item_type,
                    path,
                    title,
                    body,
                    tint_id,
                    shape.map(|s| s.as_str()),
                    x,
                    y,
                    w,
                    h,
                    next_position
                ],
            )
            .map_err(|error| error.to_string())?;
        let id = self.conn.last_insert_rowid();
        self.palette_item_by_id(id)?
            .ok_or_else(|| "Failed to reload saved palette item.".to_string())
    }

    pub fn update_palette_item_geometry(
        &mut self,
        id: i64,
        x: i64,
        y: i64,
        w: i64,
        h: i64,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE palette_items SET x = ?1, y = ?2, width = ?3, height = ?4 WHERE id = ?5",
                params![x, y, w, h, id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn update_palette_item_content(
        &mut self,
        id: i64,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE palette_items SET title = ?1, body = ?2 WHERE id = ?3",
                params![title, body, id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn update_palette_item_mark(
        &mut self,
        id: i64,
        tint_id: Option<i64>,
        shape: Option<Shape>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE palette_items SET tint_id = ?1, shape = ?2 WHERE id = ?3",
                params![tint_id, shape.map(|s| s.as_str()), id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_palette_item(&mut self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM palette_items WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn palette_item_by_id(&self, id: i64) -> Result<Option<PaletteItemRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, palette_id, item_type, path, title, body,
                        tint_id, shape, x, y, width, height, position
                 FROM palette_items WHERE id = ?1",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_row(params![id], palette_item_from_row)
            .optional()
            .map_err(|error| error.to_string())
    }

    // ── Palette Links ──────────────────────────────────────────────────────────

    pub fn list_palette_links(&self, palette_id: i64) -> Result<Vec<PaletteLinkRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, palette_id, source_item_id, target_item_id, strength, label
                 FROM palette_links
                 WHERE palette_id = ?1
                 ORDER BY id ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![palette_id], palette_link_from_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn create_palette_link(
        &mut self,
        palette_id: i64,
        source_item_id: i64,
        target_item_id: i64,
        strength: &str,
    ) -> Result<PaletteLinkRecord, String> {
        self.conn
            .execute(
                "INSERT INTO palette_links (palette_id, source_item_id, target_item_id, strength)
                 VALUES (?1, ?2, ?3, ?4)",
                params![palette_id, source_item_id, target_item_id, strength],
            )
            .map_err(|error| error.to_string())?;
        let id = self.conn.last_insert_rowid();
        Ok(PaletteLinkRecord {
            id,
            palette_id,
            source_item_id,
            target_item_id,
            strength: strength.to_string(),
            label: None,
        })
    }

    pub fn delete_palette_link(&mut self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM palette_links WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

pub fn metadata_db_path() -> PathBuf {
    glib::user_data_dir().join("lattice").join(DB_FILE_NAME)
}

fn backup_existing_metadata_if_needed(conn: &Connection, db_path: &Path) -> Result<(), String> {
    let version = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
        .map_err(|error| error.to_string())?;
    let has_existing_bytes = db_path
        .metadata()
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false);
    if version >= DB_SCHEMA_VERSION || !db_path.exists() || !has_existing_bytes {
        return Ok(());
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let backup_path = db_path.with_file_name(format!("{DB_FILE_NAME}.backup-{stamp}"));
    if backup_path.exists() {
        return Ok(());
    }
    fs::copy(db_path, &backup_path)
        .map(|_| ())
        .map_err(|error| format!("Failed to backup metadata database before migration: {error}"))
}

#[allow(dead_code)]
fn tint_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TintRecord> {
    Ok(TintRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
        position: row.get(3)?,
        is_default: row.get::<_, i64>(4)? != 0,
    })
}

#[allow(dead_code)]
fn file_mark_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileMarkRecord> {
    let shape_value: String = row.get(2)?;
    Ok(FileMarkRecord {
        file_path: PathBuf::from(row.get::<_, String>(0)?),
        tint_id: row.get(1)?,
        shape: shape_value.parse().unwrap_or(Shape::DEFAULT),
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn palette_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PaletteRecord> {
    Ok(PaletteRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn palette_place_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PalettePlaceRecord> {
    Ok(PalettePlaceRecord {
        id: row.get(0)?,
        palette_id: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        position: row.get(4)?,
    })
}

fn palette_item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PaletteItemRecord> {
    Ok(PaletteItemRecord {
        id: row.get(0)?,
        palette_id: row.get(1)?,
        item_type: row.get(2)?,
        path: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        tint_id: row.get(6)?,
        shape: shape_from_optional_lossy(row.get(7)?),
        x: row.get(8)?,
        y: row.get(9)?,
        width: row.get(10)?,
        height: row.get(11)?,
        position: row.get(12)?,
    })
}

fn palette_link_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PaletteLinkRecord> {
    Ok(PaletteLinkRecord {
        id: row.get(0)?,
        palette_id: row.get(1)?,
        source_item_id: row.get(2)?,
        target_item_id: row.get(3)?,
        strength: row.get(4)?,
        label: row.get(5)?,
    })
}

fn tag_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TagRecord> {
    Ok(TagRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
        associated_tint_id: row.get(3)?,
        associated_shape: shape_from_optional_lossy(row.get(4)?),
    })
}

fn shape_from_optional_lossy(value: Option<String>) -> Option<Shape> {
    value.and_then(|shape| shape.parse().ok())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn repeat_vars(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn remap_prefix_path(
    current_path: &str,
    old_prefix: &str,
    new_prefix: &str,
) -> Result<String, String> {
    if current_path == old_prefix {
        return Ok(new_prefix.to_string());
    }

    let current = Path::new(current_path);
    let old = Path::new(old_prefix);
    let new = Path::new(new_prefix);
    let suffix = current
        .strip_prefix(old)
        .map_err(|_| format!("Unable to remap tagged path: {current_path}"))?;
    Ok(new.join(suffix).to_string_lossy().to_string())
}

fn map_constraint_error(error: rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(code, _) if code.extended_code == 2067 => {
            "That item already exists.".to_string()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_palettes_and_places_through_project_compatibility() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        let project = store.create_project("Lattice", Some("#00e5ff")).unwrap();

        let projects = store.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Lattice");
        assert_eq!(projects[0].color.as_deref(), None);
        assert_eq!(project.color.as_deref(), None);

        let palettes = store.list_palettes().unwrap();
        assert_eq!(palettes.len(), 1);
        assert_eq!(palettes[0].name, "Lattice");

        let destination = store
            .add_project_destination(project.id, "Workspace", "/tmp/lattice-project")
            .unwrap();
        assert_eq!(destination.name, "Workspace");
        assert_eq!(destination.path, "/tmp/lattice-project");

        let destinations = store.list_project_destinations(project.id).unwrap();
        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].name, "Workspace");
        assert_eq!(destinations[0].path, "/tmp/lattice-project");

        let places = store.list_palette_places(project.id).unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].name, "Workspace");
        assert_eq!(places[0].path, "/tmp/lattice-project");
    }

    #[test]
    fn default_mark_resolves_without_explicit_row() {
        let store = MetadataStore::open_in_memory().unwrap();
        let mark = store
            .mark_for_path(Path::new("/tmp/demo/file.txt"))
            .unwrap();
        let tints = store.list_tints().unwrap();
        let default_tint = tints.iter().find(|tint| tint.is_default).unwrap();

        assert_eq!(default_tint.name, DEFAULT_TINT_NAME);
        assert_eq!(default_tint.color.as_deref(), Some(DEFAULT_TINT_COLOR));
        assert_eq!(mark.tint_id, default_tint.id);
        assert_eq!(mark.shape, Shape::Square);
        assert_eq!(mark.created_at, 0);
        assert_eq!(mark.updated_at, 0);
    }

    #[test]
    fn explicit_file_marks_can_be_set_and_cleared() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        let tint = store.create_tint("Cyan", "#00e5ff").unwrap();

        let mark = store
            .set_file_mark(Path::new("/tmp/demo/file.txt"), tint.id, Shape::Triangle)
            .unwrap();
        assert_eq!(mark.tint_id, tint.id);
        assert_eq!(mark.shape, Shape::Triangle);
        assert!(mark.created_at > 0);
        assert!(mark.updated_at > 0);

        store
            .clear_file_mark(Path::new("/tmp/demo/file.txt"))
            .unwrap();
        let reset = store
            .mark_for_path(Path::new("/tmp/demo/file.txt"))
            .unwrap();
        assert_eq!(reset.shape, Shape::Square);
        assert_ne!(reset.tint_id, tint.id);
    }

    #[test]
    fn tags_move_with_renamed_paths() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        let tag = store.ensure_tag("urgent").unwrap();
        store
            .add_tag_to_paths(tag.id, &[PathBuf::from("/tmp/demo/file.txt")])
            .unwrap();

        store
            .move_tagged_path_prefix(
                Path::new("/tmp/demo/file.txt"),
                Path::new("/tmp/demo/file-2.txt"),
            )
            .unwrap();

        let tags = store
            .tags_for_paths(&[PathBuf::from("/tmp/demo/file-2.txt")])
            .unwrap();
        assert_eq!(
            tags[&PathBuf::from("/tmp/demo/file-2.txt")][0].name,
            "urgent"
        );
    }

    #[test]
    fn deleting_prefix_cleans_nested_paths() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        let tag = store.ensure_tag("keep").unwrap();
        store
            .add_tag_to_paths(
                tag.id,
                &[
                    PathBuf::from("/tmp/demo/a.txt"),
                    PathBuf::from("/tmp/demo/folder/b.txt"),
                ],
            )
            .unwrap();

        store
            .delete_tagged_path_prefix(Path::new("/tmp/demo"))
            .unwrap();

        let remaining = store.list_paths_for_tag(tag.id).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn recent_locations_are_capped_and_sorted() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        store.record_recent_location(Path::new("/tmp/one")).unwrap();
        store.record_recent_location(Path::new("/tmp/two")).unwrap();
        store.record_recent_location(Path::new("/tmp/one")).unwrap();

        let recent = store.list_recent_locations(10).unwrap();
        assert_eq!(recent[0], PathBuf::from("/tmp/one"));
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn activity_log_preserves_item_history() {
        let store = MetadataStore::open_in_memory().unwrap();
        let items = vec![
            (
                PathBuf::from("/tmp/source/a.txt"),
                Some(PathBuf::from("/tmp/dest/a.txt")),
            ),
            (
                PathBuf::from("/tmp/source/b.txt"),
                Some(PathBuf::from("/tmp/dest/b.txt")),
            ),
        ];

        let activity_id = store
            .log_activity_with_items(
                "copy",
                2,
                "Copied 2 files",
                "/tmp/source",
                Some("/tmp/dest"),
                &[],
                &items,
            )
            .unwrap();

        let entries = store.list_recent_activity(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, activity_id);
        assert_eq!(entries[0].items.len(), 2);
        assert_eq!(entries[0].items[0].source_path, "/tmp/source/a.txt");
        assert_eq!(
            entries[0].items[1].destination_path.as_deref(),
            Some("/tmp/dest/b.txt")
        );
    }

    #[test]
    fn places_can_be_created_and_removed() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        let place = store
            .create_place("Downloads", Path::new("/tmp/downloads"))
            .unwrap();

        let places = store.list_places().unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].name, "Downloads");
        assert_eq!(places[0].folder_path, PathBuf::from("/tmp/downloads"));

        store.remove_place(place.id).unwrap();
        assert!(store.list_places().unwrap().is_empty());
    }
}
