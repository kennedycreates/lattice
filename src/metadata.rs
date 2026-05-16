use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const DB_FILE_NAME: &str = "metadata.db";
const DB_SCHEMA_VERSION: i32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: i64,
    pub name: String,
    pub root_path: PathBuf,
    pub accent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDestinationRecord {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagRecord {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
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
                "SELECT id, name, root_path, accent
                 FROM projects
                 ORDER BY name COLLATE NOCASE ASC",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], |row| {
                Ok(ProjectRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    root_path: PathBuf::from(row.get::<_, String>(2)?),
                    accent: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn create_project(
        &mut self,
        name: &str,
        root_path: &Path,
    ) -> Result<ProjectRecord, String> {
        let normalized_root = normalize_path(root_path);
        if normalized_root.is_empty() {
            return Err("Project root path cannot be empty.".to_string());
        }

        let tx = self.conn.transaction().map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO projects (name, root_path, accent) VALUES (?1, ?2, NULL)",
            params![name.trim(), normalized_root],
        )
        .map_err(map_constraint_error)?;
        let project_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO project_destinations (project_id, name, relative_path)
             VALUES (?1, 'Root', '')",
            params![project_id],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;

        self.project_by_id(project_id)?
            .ok_or_else(|| "Failed to reload saved project.".to_string())
    }

    pub fn list_project_destinations(
        &self,
        project_id: i64,
    ) -> Result<Vec<ProjectDestinationRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, project_id, name, relative_path
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
                    relative_path: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
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
            "INSERT INTO recent_locations (folder_path, last_visited_unix)
             VALUES (?1, CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER))
             ON CONFLICT(folder_path) DO UPDATE
             SET last_visited_unix = excluded.last_visited_unix",
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

    fn initialize(&mut self) -> Result<(), String> {
        let version = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .map_err(|error| error.to_string())?;
        if version >= DB_SCHEMA_VERSION {
            return Ok(());
        }

        self.conn
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;

                CREATE TABLE IF NOT EXISTS projects (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    root_path TEXT NOT NULL UNIQUE,
                    accent TEXT
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
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    name TEXT NOT NULL,
                    relative_path TEXT NOT NULL DEFAULT ''
                );

                CREATE TABLE IF NOT EXISTS recent_locations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    folder_path TEXT NOT NULL UNIQUE,
                    last_visited_unix INTEGER NOT NULL
                );
                ",
            )
            .map_err(|error| error.to_string())?;

        self.conn
            .pragma_update(None, "user_version", DB_SCHEMA_VERSION)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn project_by_id(&self, id: i64) -> Result<Option<ProjectRecord>, String> {
        self.conn
            .query_row(
                "SELECT id, name, root_path, accent FROM projects WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ProjectRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        root_path: PathBuf::from(row.get::<_, String>(2)?),
                        accent: row.get(3)?,
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
    fn creates_projects_and_root_destination() {
        let mut store = MetadataStore::open_in_memory().unwrap();
        let project = store
            .create_project("Lattice", Path::new("/tmp/lattice-project"))
            .unwrap();

        let projects = store.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Lattice");
        assert_eq!(projects[0].root_path, PathBuf::from("/tmp/lattice-project"));
        assert_eq!(project.accent, None);

        let destinations = store.list_project_destinations(project.id).unwrap();
        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].name, "Root");
        assert_eq!(destinations[0].relative_path, "");
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
}
