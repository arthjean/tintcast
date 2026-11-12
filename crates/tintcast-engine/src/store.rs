use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic;
use crate::data_dir::DataDir;
use crate::error::EngineError;
use crate::operation::{Operation, ResourceState, sha256_hex};

pub const VERSION: u32 = 1;
pub const HISTORY_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub sequence: u64,
    pub resource: String,
    pub integration: String,
    pub operation: Operation,
    pub original: ResourceState,
    pub applied: ResourceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<ResourceState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_backup: Option<String>,
    pub first_touched: Timestamp,
    pub last_applied: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedValue {
    pub resource: String,
    pub value: ResourceState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub theme_id: String,
    pub theme_name: String,
    pub wallpaper: usize,
    pub at: Timestamp,
    pub integrations: Vec<String>,
    pub applied: Vec<AppliedValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Applied,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalStep {
    pub integration: String,
    pub resource: String,
    pub summary: String,
    pub operation: Operation,
    pub before: ResourceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_backup: Option<String>,
    pub state: StepState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalRun {
    pub version: u32,
    pub run_id: String,
    pub theme_id: String,
    pub theme_name: String,
    pub started: Timestamp,
    pub steps: Vec<JournalStep>,
}

#[derive(Serialize, Deserialize)]
struct Versioned<T> {
    version: u32,
    entries: Vec<T>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Ledger {
    entries: BTreeMap<String, LedgerEntry>,
}

impl Ledger {
    pub(crate) fn get(&self, resource: &str) -> Option<&LedgerEntry> {
        self.entries.get(resource)
    }

    pub(crate) fn get_mut(&mut self, resource: &str) -> Option<&mut LedgerEntry> {
        self.entries.get_mut(resource)
    }

    pub(crate) fn insert(&mut self, entry: LedgerEntry) {
        self.entries.insert(entry.resource.clone(), entry);
    }

    pub(crate) fn remove(&mut self, resource: &str) -> Option<LedgerEntry> {
        self.entries.remove(resource)
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.entries.values().map(|e| e.sequence).max().unwrap_or(0) + 1
    }

    pub(crate) fn newest_first(&self) -> Vec<LedgerEntry> {
        let mut entries: Vec<_> = self.entries.values().cloned().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.sequence));
        entries
    }

    pub(crate) fn oldest_first(&self) -> Vec<LedgerEntry> {
        let mut entries = self.newest_first();
        entries.reverse();
        entries
    }
}

#[derive(Debug, Default)]
pub(crate) struct JournalScan {
    pub runs: Vec<JournalRun>,
    pub corrupt: Vec<(String, PathBuf)>,
}

#[derive(Debug, Clone)]
pub(crate) struct Store {
    dir: DataDir,
}

fn pretty<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

fn read_optional(path: &std::path::Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<Vec<T>, String> {
    let file: Versioned<T> = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if file.version != VERSION {
        return Err(format!("version {} is not version {VERSION}", file.version));
    }
    Ok(file.entries)
}

impl Store {
    pub(crate) fn new(dir: DataDir) -> Self {
        Self { dir }
    }

    pub(crate) fn dir(&self) -> &DataDir {
        &self.dir
    }

    pub(crate) fn load_ledger(&self) -> Result<Ledger, EngineError> {
        let path = self.dir.ledger();
        let unreadable = |reason: String| EngineError::LedgerUnreadable {
            path: path.clone(),
            reason,
        };
        let Some(bytes) = read_optional(&path).map_err(|e| unreadable(e.to_string()))? else {
            return Ok(Ledger::default());
        };
        let entries: Vec<LedgerEntry> = decode(&bytes).map_err(unreadable)?;
        let mut ledger = Ledger::default();
        for entry in entries {
            ledger.insert(entry);
        }
        Ok(ledger)
    }

    pub(crate) fn save_ledger(&self, ledger: &Ledger) -> Result<(), EngineError> {
        let path = self.dir.ledger();
        let file = Versioned {
            version: VERSION,
            entries: ledger.oldest_first(),
        };
        write_atomic(&path, &pretty(&file)).map_err(EngineError::io(path))
    }

    pub(crate) fn load_history(&self) -> Result<Vec<HistoryEntry>, EngineError> {
        let path = self.dir.history();
        let Some(bytes) = read_optional(&path).map_err(EngineError::io(&path))? else {
            return Ok(Vec::new());
        };
        match decode(&bytes) {
            Ok(entries) => Ok(entries),
            Err(_) => {
                let aside = self.dir.root().join("history.corrupt.json");
                fs::rename(&path, &aside).map_err(EngineError::io(&path))?;
                Ok(Vec::new())
            }
        }
    }

    pub(crate) fn save_history(&self, history: &[HistoryEntry]) -> Result<(), EngineError> {
        let path = self.dir.history();
        let start = history.len().saturating_sub(HISTORY_LIMIT);
        let file = Versioned {
            version: VERSION,
            entries: history[start..].to_vec(),
        };
        write_atomic(&path, &pretty(&file)).map_err(EngineError::io(path))
    }

    fn journal_path(&self, run_id: &str) -> PathBuf {
        self.dir.journal().join(format!("{run_id}.json"))
    }

    pub(crate) fn save_journal(&self, run: &JournalRun) -> Result<(), EngineError> {
        let path = self.journal_path(&run.run_id);
        write_atomic(&path, &pretty(run)).map_err(EngineError::io(path))
    }

    pub(crate) fn remove_journal(&self, run_id: &str) -> Result<(), EngineError> {
        let path = self.journal_path(run_id);
        match fs::remove_file(&path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                Err(EngineError::io(path)(error))
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn scan_journals(&self) -> Result<JournalScan, EngineError> {
        let folder = self.dir.journal();
        let mut scan = JournalScan::default();
        let listing = fs::read_dir(&folder).map_err(EngineError::io(&folder))?;
        let mut paths: Vec<PathBuf> = listing.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            let Some(run_id) = name.strip_suffix(".json") else {
                continue;
            };
            if run_id.ends_with(".corrupt") || name.starts_with('.') {
                continue;
            }
            let parsed = fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<JournalRun>(&bytes).map_err(|e| e.to_string())
                })
                .and_then(|run| {
                    if run.version == VERSION {
                        Ok(run)
                    } else {
                        Err(format!("version {} is not version {VERSION}", run.version))
                    }
                });
            match parsed {
                Ok(run) => scan.runs.push(run),
                Err(_) => {
                    let aside = folder.join(format!("{run_id}.corrupt.json"));
                    fs::rename(&path, &aside).map_err(EngineError::io(&path))?;
                    scan.corrupt.push((run_id.to_owned(), aside));
                }
            }
        }
        scan.runs.sort_by_key(|r| r.started);
        Ok(scan)
    }

    pub(crate) fn referenced_files(&self) -> Result<Vec<PathBuf>, EngineError> {
        fn add_state(state: &ResourceState, out: &mut Vec<PathBuf>) {
            if let ResourceState::Wallpaper { image } = state {
                out.push(image.clone());
            }
        }
        fn add_operation(operation: &Operation, out: &mut Vec<PathBuf>) {
            if let Operation::SetWallpaper { image, .. } = operation {
                out.push(image.clone());
            }
        }
        let mut out = Vec::new();
        for entry in self.load_ledger()?.oldest_first() {
            add_operation(&entry.operation, &mut out);
            add_state(&entry.original, &mut out);
            add_state(&entry.applied, &mut out);
            if let Some(pending) = &entry.pending {
                add_state(pending, &mut out);
            }
        }
        let history = self.dir.history();
        if let Some(bytes) = read_optional(&history).map_err(EngineError::io(&history))? {
            let entries: Vec<HistoryEntry> = decode(&bytes).unwrap_or_default();
            for entry in entries {
                for value in &entry.applied {
                    add_state(&value.value, &mut out);
                }
            }
        }
        let folder = self.dir.journal();
        let listing = fs::read_dir(&folder).map_err(EngineError::io(&folder))?;
        for path in listing.filter_map(|e| e.ok().map(|e| e.path())) {
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(run) = serde_json::from_slice::<JournalRun>(&bytes) else {
                continue;
            };
            for step in &run.steps {
                add_operation(&step.operation, &mut out);
                add_state(&step.before, &mut out);
            }
        }
        Ok(out)
    }

    pub(crate) fn load_journal(&self, run_id: &str) -> Result<JournalRun, EngineError> {
        let well_formed = !run_id.is_empty()
            && run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !well_formed {
            return Err(EngineError::UnknownRun(run_id.to_owned()));
        }
        let path = self.journal_path(run_id);
        let bytes = read_optional(&path)
            .map_err(EngineError::io(&path))?
            .ok_or_else(|| EngineError::UnknownRun(run_id.to_owned()))?;
        serde_json::from_slice(&bytes).map_err(|_| EngineError::UnknownRun(run_id.to_owned()))
    }

    pub(crate) fn backup(&self, bytes: &[u8]) -> Result<String, EngineError> {
        let sha = sha256_hex(bytes);
        let path = self.dir.backups().join(&sha);
        if !path.exists() {
            write_atomic(&path, bytes).map_err(EngineError::io(&path))?;
        }
        Ok(sha)
    }

    pub(crate) fn read_backup(&self, sha: &str) -> Result<Vec<u8>, String> {
        let path = self.dir.backups().join(sha);
        let bytes = fs::read(&path)
            .map_err(|e| format!("Tintcast's copy {} can't be read: {e}", path.display()))?;
        if sha256_hex(&bytes) != sha {
            return Err(format!(
                "Tintcast's copy {} does not match its checksum",
                path.display()
            ));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().unwrap();
        let dir = DataDir::at(temp.path().join("Tintcast")).unwrap();
        (temp, Store::new(dir))
    }

    fn entry(sequence: u64) -> LedgerEntry {
        LedgerEntry {
            sequence,
            resource: format!("r{sequence}"),
            integration: "test".into(),
            operation: Operation::SetWallpaperPosition {
                position: tintcast_theme::WallpaperPosition::Fit,
            },
            original: ResourceState::Absent,
            applied: ResourceState::Absent,
            pending: None,
            file_backup: None,
            first_touched: Timestamp::now(),
            last_applied: Timestamp::now(),
            error: None,
        }
    }

    fn history_entry(n: usize) -> HistoryEntry {
        HistoryEntry {
            theme_id: format!("t{n}"),
            theme_name: format!("T{n}"),
            wallpaper: 0,
            at: Timestamp::now(),
            integrations: vec!["test".into()],
            applied: Vec::new(),
        }
    }

    #[test]
    fn ledger_and_history_carry_version_one() {
        let (_temp, store) = store();
        let mut ledger = Ledger::default();
        ledger.insert(entry(1));
        store.save_ledger(&ledger).unwrap();
        store.save_history(&[history_entry(1)]).unwrap();
        for path in [store.dir().ledger(), store.dir().history()] {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(value["version"], 1);
        }
        assert!(store.load_ledger().unwrap().get("r1").is_some());
    }

    #[test]
    fn history_keeps_the_latest_fifty_entries() {
        let (_temp, store) = store();
        let history: Vec<_> = (0..60).map(history_entry).collect();
        store.save_history(&history).unwrap();
        let loaded = store.load_history().unwrap();
        assert_eq!(loaded.len(), 50);
        assert_eq!(loaded[0].theme_id, "t10");
        assert_eq!(loaded[49].theme_id, "t59");
    }

    #[test]
    fn a_ledger_that_does_not_parse_blocks_with_a_message_naming_it() {
        let (_temp, store) = store();
        fs::write(store.dir().ledger(), b"{ not json").unwrap();
        let error = store.load_ledger().unwrap_err();
        assert!(matches!(error, EngineError::LedgerUnreadable { .. }));
        assert!(
            error
                .to_string()
                .contains(&store.dir().ledger().display().to_string())
        );
        assert_eq!(fs::read(store.dir().ledger()).unwrap(), b"{ not json");
    }

    #[test]
    fn a_ledger_with_another_version_is_unreadable() {
        let (_temp, store) = store();
        fs::write(store.dir().ledger(), br#"{"version":2,"entries":[]}"#).unwrap();
        assert!(store.load_ledger().is_err());
    }

    #[test]
    fn backups_are_stored_once_by_content_hash() {
        let (_temp, store) = store();
        let a = store.backup(b"same").unwrap();
        let b = store.backup(b"same").unwrap();
        assert_eq!(a, b);
        assert_eq!(fs::read_dir(store.dir().backups()).unwrap().count(), 1);
        assert_eq!(store.read_backup(&a).unwrap(), b"same");
        fs::write(store.dir().backups().join(&a), b"tampered").unwrap();
        assert!(store.read_backup(&a).is_err());
    }
}
