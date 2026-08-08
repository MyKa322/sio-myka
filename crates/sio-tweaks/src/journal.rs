//! The undo log.
//!
//! Every tweak application writes one file recording what each action found *before* it
//! changed anything. Reverting replays those captures. Without this the app could
//! toggle settings but never honestly put them back, because "the default value" and
//! "what was actually there" are different things.
//!
//! One file per entry rather than one append-only log: marking a single entry reverted
//! is then a whole-file rewrite instead of a rewrite-in-place, which cannot corrupt the
//! other entries if it fails halfway.

use sio_core::error::{Error, Result};
use sio_core::tweak::JournalEntry;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct JournalStore {
    dir: PathBuf,
}

/// `%LOCALAPPDATA%\SIO\journal`.
///
/// Local rather than roaming: the journal describes *this* machine's registry, and
/// following a user to another PC would offer to revert changes never made there.
pub fn default_dir() -> Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Error::Other("LOCALAPPDATA is not set".into()))?;
    Ok(PathBuf::from(base).join("SIO").join("journal"))
}

/// Build a file name that is stable for a given entry, so marking it reverted
/// overwrites the same file rather than creating a second copy.
fn file_name(entry: &JournalEntry) -> String {
    let safe: String = entry
        .tweak_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{:013}-{safe}.json", entry.applied_at)
}

impl JournalStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn open_default() -> Result<Self> {
        Ok(Self::new(default_dir()?))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write an entry.
    ///
    /// Called immediately after the prior state is captured and before the caller
    /// reports success, so a crash mid-tweak still leaves a record of what was touched.
    pub async fn write(&self, entry: &JournalEntry) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let path = self.dir.join(file_name(entry));
        tokio::fs::write(&path, serde_json::to_string_pretty(entry)?).await?;
        Ok(path)
    }

    /// Every entry, newest first.
    ///
    /// A malformed file is skipped with a warning rather than failing the listing: one
    /// corrupt entry must not hide the rest of the history.
    pub async fn list(&self) -> Result<Vec<JournalEntry>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        let mut out = Vec::new();

        while let Some(item) = entries.next_entry().await? {
            let path = item.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match tokio::fs::read_to_string(&path).await {
                Ok(text) => match serde_json::from_str::<JournalEntry>(&text) {
                    Ok(entry) => out.push(entry),
                    Err(e) => {
                        tracing::warn!("skipping malformed journal entry {}: {e}", path.display())
                    }
                },
                Err(e) => tracing::warn!("could not read {}: {e}", path.display()),
            }
        }

        out.sort_by_key(|e| std::cmp::Reverse(e.applied_at));
        Ok(out)
    }

    /// Entries that have not been reverted yet.
    pub async fn active(&self) -> Result<Vec<JournalEntry>> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .filter(|e| !e.is_reverted())
            .collect())
    }

    /// Find the newest entry for a tweak that is still in effect.
    pub async fn newest_active_for(&self, tweak_id: &str) -> Result<Option<JournalEntry>> {
        Ok(self
            .active()
            .await?
            .into_iter()
            .find(|e| e.tweak_id == tweak_id))
    }

    /// Overwrite an entry, e.g. to record that it was reverted.
    pub async fn update(&self, entry: &JournalEntry) -> Result<()> {
        self.write(entry).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sio_core::tweak::{AppliedAction, Hive, PriorValue, RegistryValue};

    fn temp_store(tag: &str) -> JournalStore {
        let dir = std::env::temp_dir()
            .join("sio-journal-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        JournalStore::new(dir)
    }

    fn entry(tweak_id: &str, applied_at: u64) -> JournalEntry {
        JournalEntry::new(
            tweak_id,
            applied_at,
            vec![AppliedAction::Registry {
                hive: Hive::Hkcu,
                path: r"Software\SioTest".into(),
                name: "Value".into(),
                prior: PriorValue::Present(RegistryValue::Dword(1)),
            }],
        )
    }

    #[tokio::test]
    async fn writes_and_reads_back_an_entry() {
        let store = temp_store("roundtrip");
        let original = entry("privacy.telemetry.disable", 1_700_000_000_000);
        store.write(&original).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0], original,
            "the captured prior state must survive exactly"
        );
    }

    #[tokio::test]
    async fn entries_are_listed_newest_first() {
        let store = temp_store("order");
        store.write(&entry("a", 1000)).await.unwrap();
        store.write(&entry("b", 3000)).await.unwrap();
        store.write(&entry("c", 2000)).await.unwrap();

        let ids: Vec<_> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.tweak_id)
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[tokio::test]
    async fn marking_reverted_overwrites_rather_than_duplicating() {
        let store = temp_store("revert");
        let mut original = entry("interface.taskbar.align_left", 1_700_000_000_000);
        store.write(&original).await.unwrap();

        original.reverted_at = Some(1_700_000_100_000);
        store.update(&original).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(
            listed.len(),
            1,
            "updating must not leave a second file behind"
        );
        assert!(listed[0].is_reverted());
    }

    #[tokio::test]
    async fn active_excludes_reverted_entries() {
        let store = temp_store("active");
        store.write(&entry("still-on", 2000)).await.unwrap();

        let mut undone = entry("undone", 1000);
        undone.reverted_at = Some(1500);
        store.write(&undone).await.unwrap();

        let active = store.active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].tweak_id, "still-on");
    }

    #[tokio::test]
    async fn newest_active_entry_wins_when_a_tweak_was_applied_twice() {
        // Re-applying a tweak writes a second entry. Reverting must use the newest,
        // because that one holds the state immediately before the current situation.
        let store = temp_store("twice");
        store.write(&entry("t", 1000)).await.unwrap();
        store.write(&entry("t", 5000)).await.unwrap();

        let found = store.newest_active_for("t").await.unwrap().unwrap();
        assert_eq!(found.applied_at, 5000);
    }

    #[tokio::test]
    async fn a_missing_directory_lists_as_empty_rather_than_erroring() {
        let store = JournalStore::new(std::env::temp_dir().join("sio-journal-never-created"));
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_corrupt_entry_does_not_hide_the_others() {
        let store = temp_store("corrupt");
        store.write(&entry("good", 1000)).await.unwrap();
        tokio::fs::write(store.dir().join("0000000000002-broken.json"), "{ not json")
            .await
            .unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].tweak_id, "good");
    }

    #[tokio::test]
    async fn file_names_are_sortable_and_safe() {
        // Zero-padded so lexical order matches chronological order in a file listing,
        // and free of anything a path could choke on.
        let name = file_name(&entry("privacy.telemetry.disable", 42));
        assert!(name.starts_with("0000000000042-"), "got {name}");
        assert!(!name.contains('/') && !name.contains('\\'));

        let hostile = file_name(&entry("../../evil id", 1));
        assert!(
            !hostile.contains('/') && !hostile.contains('\\'),
            "got {hostile}"
        );
    }
}
