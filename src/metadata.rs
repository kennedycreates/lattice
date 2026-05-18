use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const DB_FILE_NAME: &str = "metadata.db";
const DB_SCHEMA_VERSION: i32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: i64,
    pub name: String,
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
pub struct TagRecord {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
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

pub struct MetadataStore {
    conn: Connection,
}

impl MetadataStore {
    pub fn open() -> Result<Self, String> {
        let db_path = metadata_db_path();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create metadata directory: {error}"))?;
        }

        let conn = Connection::open(&db_path)
            .map_err(|error| format!("Failed to open metadata database: {error}"))?;
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
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, color
                 FROM projects
                 ORDER BY name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], |row| {
                Ok(ProjectRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn create_project(
        &mut self,
        name: &str,
        color: Option<&str>,
    ) -> Result<ProjectRecord, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Project name cannot be empty.".to_string());
        }

        self.conn
            .execute(
                "INSERT INTO projects (name, color) VALUES (?1, ?2)",
                params![trimmed, color],
            )
            .map_err(map_constraint_error)?;
        let project_id = self.conn.last_insert_rowid();

        self.project_by_id(project_id)?
            .ok_or_else(|| "Failed to reload saved project.".to_string())
    }

    pub fn rename_project(&mut self, id: i64, name: &str) -> Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Project name cannot be empty.".to_string());
        }
        self.conn
            .execute(
                "UPDATE projects SET name = ?1 WHERE id = ?2",
                params![trimmed, id],
            )
            .map_err(map_constraint_error)?;
        Ok(())
    }

    pub fn update_project_color(&mut self, id: i64, color: Option<&str>) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE projects SET color = ?1 WHERE id = ?2",
                params![color, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_project_destinations(
        &self,
        project_id: i64,
    ) -> Result<Vec<ProjectDestinationRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, project_id, name, path
                 FROM project_destinations
                 WHERE project_id = ?1
                 ORDER BY id ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok(ProjectDestinationRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    name: row.get(2)?,
                    path: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn delete_project(&mut self, project_id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM projects WHERE id = ?1", params![project_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn add_project_destination(
        &mut self,
        project_id: i64,
        name: &str,
        path: &str,
    ) -> Result<ProjectDestinationRecord, String> {
        self.conn
            .execute(
                "INSERT INTO project_destinations (project_id, name, path)
                 VALUES (?1, ?2, ?3)",
                params![project_id, name.trim(), path.trim()],
            )
            .map_err(map_constraint_error)?;
        let id = self.conn.last_insert_rowid();
        Ok(ProjectDestinationRecord {
            id,
            project_id,
            name: name.trim().to_string(),
            path: path.trim().to_string(),
        })
    }

    pub fn remove_project_destination(&mut self, destination_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM project_destinations WHERE id = ?1",
                params![destination_id],
            )
            .map_err(|e| e.to_string())?;
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

    pub fn list_tags(&self) -> Result<Vec<TagRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, color
                 FROM tags
                 ORDER BY name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(TagRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                })
            })
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

        let color = default_tag_color(trimmed);
        self.conn
            .execute(
                "INSERT INTO tags (name, color) VALUES (?1, ?2)",
                params![trimmed, color],
            )
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
        if matches.is_empty() {
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
            "SELECT ft.file_path, t.id, t.name, t.color
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

    pub fn update_tag_color(&mut self, id: i64, color: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE tags SET color = ?1 WHERE id = ?2",
                params![color, id],
            )
            .map_err(|e| e.to_string())?;
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

        let version = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .map_err(|error| error.to_string())?;
        if version >= DB_SCHEMA_VERSION {
            self.ensure_indexes()?;
            return Ok(());
        }

        // v6: projects no longer tied to a folder; destinations store absolute paths.
        // Drop and recreate project tables — pre-1.0, data loss is acceptable.
        if version > 0 {
            self.conn
                .execute_batch(
                    "DROP TABLE IF EXISTS project_destinations;
                     DROP TABLE IF EXISTS projects;",
                )
                .map_err(|error| error.to_string())?;
        }

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
                    color TEXT
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

                CREATE TABLE IF NOT EXISTS places (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    folder_path TEXT NOT NULL UNIQUE,
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
                ",
            )
            .map_err(|error| error.to_string())?;

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
                CREATE INDEX IF NOT EXISTS idx_recent_locations_last_visited ON recent_locations(last_visited_unix DESC);
                CREATE INDEX IF NOT EXISTS idx_activity_log_timestamp ON activity_log(timestamp_ms DESC);
                CREATE INDEX IF NOT EXISTS idx_activity_log_items_activity_id ON activity_log_items(activity_id, item_index, id);
                ",
            )
            .map_err(|error| error.to_string())
    }

    fn project_by_id(&self, id: i64) -> Result<Option<ProjectRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, color FROM projects WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ProjectRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
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
                "SELECT id, name, color FROM tags WHERE id = ?1",
                params![id],
                |row| {
                    Ok(TagRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn find_tag_by_name(&self, name: &str) -> Result<Option<TagRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, color FROM tags WHERE lower(name) = lower(?1)",
                params![name],
                |row| {
                    Ok(TagRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                    })
                },
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
}

pub fn metadata_db_path() -> PathBuf {
    glib::user_data_dir().join("lattice").join(DB_FILE_NAME)
}

fn default_tag_color(name: &str) -> &'static str {
    const PALETTE: [&str; 8] = [
        "#00e5ff", "#10b981", "#f59e0b", "#ff4b6e", "#a855f7", "#38bdf8", "#22c55e", "#f97316",
    ];

    let index = name
        .bytes()
        .fold(0usize, |acc, value| acc.wrapping_add(value as usize))
        % PALETTE.len();
    PALETTE[index]
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
    fn creates_projects_and_destinations() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        let project = store.create_project("Lattice", Some("#00e5ff")).unwrap();

        let projects = store.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Lattice");
        assert_eq!(projects[0].color.as_deref(), Some("#00e5ff"));
        assert_eq!(project.color.as_deref(), Some("#00e5ff"));

        let destination = store
            .add_project_destination(project.id, "Workspace", "/tmp/lattice-project")
            .unwrap();
        assert_eq!(destination.name, "Workspace");
        assert_eq!(destination.path, "/tmp/lattice-project");

        let destinations = store.list_project_destinations(project.id).unwrap();
        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].name, "Workspace");
        assert_eq!(destinations[0].path, "/tmp/lattice-project");
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
