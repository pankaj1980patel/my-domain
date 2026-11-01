//! A tiny persistent key-value store: one flat JSON object in a file.
//!
//! Every platform needs somewhere to remember the session (server URL,
//! username) across restarts. Desktop had its own copy of this; Android had
//! none and forgot the login on every restart. Platforms now use `FileKv`
//! from `Platform::kv_get`/`kv_set`, pointing it at their app data directory.
//!
//! Writes are atomic: the new contents go to `<file>.tmp`, which is then
//! renamed over the real file. A rename within one directory is atomic on
//! every OS we target, so a crash mid-write leaves either the old or the new
//! file, never a half-written one. (Node analogy: `write-file-atomic`.)
//!
//! Small and infrequent by design (a few keys, written at login). Not a
//! database.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;

pub struct FileKv {
    path: PathBuf,
    /// Serializes read-modify-write cycles from different threads.
    lock: Mutex<()>,
}

impl FileKv {
    /// Store at `path`; parent directories are created on first write.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FileKv {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    /// Store as `session.json` inside `dir`.
    pub fn in_dir(dir: impl AsRef<Path>) -> Self {
        Self::new(dir.as_ref().join("session.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let _guard = self.lock.lock();
        self.read().remove(key)
    }

    pub fn set(&self, key: &str, value: &str) -> std::io::Result<()> {
        let _guard = self.lock.lock();
        let mut map = self.read();
        map.insert(key.to_string(), value.to_string());
        self.write(&map)
    }

    pub fn remove(&self, key: &str) -> std::io::Result<()> {
        let _guard = self.lock.lock();
        let mut map = self.read();
        if map.remove(key).is_some() {
            self.write(&map)?;
        }
        Ok(())
    }

    /// Missing or unreadable file = empty store (first run, or corrupted by
    /// something outside our control: start fresh rather than fail).
    fn read(&self) -> BTreeMap<String, String> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn write(&self, map: &BTreeMap<String, String>) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec_pretty(map).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mdcore-kv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let kv = FileKv::in_dir(temp_dir("empty"));
        assert_eq!(kv.get("server_url"), None);
    }

    #[test]
    fn values_survive_a_new_instance() {
        let dir = temp_dir("persist");
        FileKv::in_dir(&dir).set("username", "alice").unwrap();
        // A "restart": a fresh FileKv on the same directory.
        assert_eq!(
            FileKv::in_dir(&dir).get("username").as_deref(),
            Some("alice")
        );
    }

    #[test]
    fn set_overwrites_and_remove_deletes() {
        let kv = FileKv::in_dir(temp_dir("overwrite"));
        kv.set("k", "1").unwrap();
        kv.set("k", "2").unwrap();
        kv.set("other", "x").unwrap();
        assert_eq!(kv.get("k").as_deref(), Some("2"));
        kv.remove("k").unwrap();
        assert_eq!(kv.get("k"), None);
        assert_eq!(kv.get("other").as_deref(), Some("x"));
    }

    #[test]
    fn corrupted_file_is_treated_as_empty_and_then_repaired() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("session.json"), b"{ not json").unwrap();
        let kv = FileKv::in_dir(&dir);
        assert_eq!(kv.get("k"), None);
        kv.set("k", "v").unwrap();
        assert_eq!(kv.get("k").as_deref(), Some("v"));
    }

    #[test]
    fn no_temp_file_is_left_behind() {
        let dir = temp_dir("tmp");
        let kv = FileKv::in_dir(&dir);
        kv.set("k", "v").unwrap();
        assert!(!dir.join("session.json.tmp").exists());
    }

    #[test]
    fn concurrent_writers_do_not_lose_keys() {
        let kv = std::sync::Arc::new(FileKv::in_dir(temp_dir("concurrent")));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let kv = kv.clone();
                std::thread::spawn(move || kv.set(&format!("k{i}"), "v").unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        for i in 0..8 {
            assert_eq!(kv.get(&format!("k{i}")).as_deref(), Some("v"), "k{i}");
        }
    }
}
