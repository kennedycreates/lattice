use gtk::gdk;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Pixel size used for the cached PNG.
const CACHE_SIZE: i32 = 128;

/// Maximum thumbnail loads running concurrently per pane.
const MAX_CONCURRENT: u32 = 6;

#[derive(Clone, Debug)]
pub enum ThumbnailKind {
    Image,
    Video,
    Audio,
}

/// A handle to the Stack/Picture widgets inside a media card, passed back to
/// the loader so it can swap in the real thumbnail once it arrives.
#[derive(Clone)]
pub struct ThumbnailTarget {
    pub path: PathBuf,
    pub mtime: i64,
    pub stack: gtk::Stack,
    pub picture: gtk::Picture,
    pub kind: ThumbnailKind,
}

struct PendingLoad {
    path: PathBuf,
    mtime: i64,
    stack: gtk::Stack,
    picture: gtk::Picture,
    kind: ThumbnailKind,
    /// The generation counter captured when this load was queued.
    gen: u64,
}

/// Per-pane thumbnail loader. Cheap to clone (all state behind Rc).
#[derive(Clone)]
pub struct ThumbnailLoader {
    generation: Rc<Cell<u64>>,
    active: Rc<Cell<u32>>,
    queue: Rc<RefCell<VecDeque<PendingLoad>>>,
    cache_dir: PathBuf,
}

impl ThumbnailLoader {
    pub fn new() -> Self {
        Self {
            generation: Rc::new(Cell::new(0)),
            active: Rc::new(Cell::new(0)),
            queue: Rc::new(RefCell::new(VecDeque::new())),
            cache_dir: glib::user_cache_dir().join("lattice").join("thumbs"),
        }
    }

    /// Invalidate any in-flight or queued loads. Safe to call from main thread.
    pub fn cancel(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.queue.borrow_mut().clear();
    }

    /// Enqueue thumbnail targets and immediately start up to MAX_CONCURRENT loads.
    pub fn submit(&self, targets: Vec<ThumbnailTarget>) {
        let gen = self.generation.get();
        {
            let mut q = self.queue.borrow_mut();
            for t in targets {
                q.push_back(PendingLoad {
                    path: t.path,
                    mtime: t.mtime,
                    stack: t.stack,
                    picture: t.picture,
                    kind: t.kind,
                    gen,
                });
            }
        }
        while self.active.get() < MAX_CONCURRENT && self.pump() {}
    }

    /// Pop one item from the queue and spawn an async task for it.
    /// Returns false if the queue was empty.
    fn pump(&self) -> bool {
        let pending = match self.queue.borrow_mut().pop_front() {
            Some(p) => p,
            None => return false,
        };

        self.active.set(self.active.get() + 1);

        let loader = self.clone();
        let cache_dir = self.cache_dir.clone();
        let generation = self.generation.clone();

        glib::MainContext::default().spawn_local(async move {
            let path = pending.path.clone();
            let mtime = pending.mtime;
            let kind = pending.kind.clone();
            let gen_at_start = pending.gen;

            // Run the expensive decode+scale+encode on a thread-pool thread.
            // Only path, mtime, cache_dir, and kind are passed — all are Send.
            let result =
                gio::spawn_blocking(move || ensure_cached(&path, mtime, &cache_dir, &kind)).await;

            // Back on the main thread. Skip the UI update if the pane has
            // already navigated away (generation changed or load was cancelled).
            if generation.get() == gen_at_start {
                if let Ok(Some(cache_path)) = result {
                    if let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file(&cache_path) {
                        let texture = gdk::Texture::for_pixbuf(&pixbuf);
                        pending.picture.set_paintable(Some(&texture));
                        pending.stack.set_visible_child_name("thumb");
                    }
                }
            }

            loader.active.set(loader.active.get() - 1);
            // Start the next queued item, if any.
            loader.pump();
        });

        true
    }
}

/// Called from a thread-pool thread. Returns the path to the cached PNG,
/// generating it first if necessary.
fn ensure_cached(
    source: &Path,
    mtime: i64,
    cache_dir: &Path,
    kind: &ThumbnailKind,
) -> Option<PathBuf> {
    let dest = cache_path(source, mtime, cache_dir);

    if dest.exists() {
        return Some(dest);
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    match kind {
        ThumbnailKind::Image => {
            let pixbuf =
                gdk_pixbuf::Pixbuf::from_file_at_scale(source, CACHE_SIZE, CACHE_SIZE, true)
                    .ok()?;
            let bytes = pixbuf.save_to_bufferv("png", &[]).ok()?;
            std::fs::write(&dest, bytes).ok()?;
        }
        ThumbnailKind::Video | ThumbnailKind::Audio => {
            let status = std::process::Command::new("ffmpegthumbnailer")
                .args([
                    "-i",
                    source.to_str()?,
                    "-o",
                    dest.to_str()?,
                    "-s",
                    &CACHE_SIZE.to_string(),
                    "-t",
                    "10%",
                    "-q",
                    "4",
                    "-f",
                ])
                .status()
                .ok()?;
            if !status.success() {
                return None;
            }
        }
    }

    Some(dest)
}

/// Stable cache filename: `{hash}_{mtime}.png`.
/// Stale entries (different mtime) are simply left on disk and ignored; a
/// separate cleanup pass could remove them, but they are tiny.
fn cache_path(source: &Path, mtime: i64, cache_dir: &Path) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    let hash = hasher.finish();
    cache_dir.join(format!("{hash:016x}_{mtime}.png"))
}
