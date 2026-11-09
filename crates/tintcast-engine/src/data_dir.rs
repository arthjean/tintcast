use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum DataDirError {
    #[error(
        "%LOCALAPPDATA% is not set to a full path, so Tintcast can't open its data folder %LOCALAPPDATA%\\Tintcast."
    )]
    LocalAppDataUnset,
    #[error("Tintcast can't write to its data folder {}: {source}", path.display())]
    NotWritable { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    pub const FOLDER: &'static str = "Tintcast";

    pub fn open() -> Result<Self, DataDirError> {
        Self::open_in(std::env::var_os("LOCALAPPDATA"))
    }

    pub fn open_in(local_app_data: Option<OsString>) -> Result<Self, DataDirError> {
        let base = local_app_data
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(DataDirError::LocalAppDataUnset)?;
        Self::at(base.join(Self::FOLDER))
    }

    pub fn at(root: impl Into<PathBuf>) -> Result<Self, DataDirError> {
        let dir = Self { root: root.into() };
        dir.prepare()?;
        Ok(dir)
    }

    fn prepare(&self) -> Result<(), DataDirError> {
        let not_writable = |path: &Path| {
            let path = path.to_path_buf();
            move |source| DataDirError::NotWritable { path, source }
        };
        fs::create_dir_all(&self.root).map_err(not_writable(&self.root))?;
        for folder in self.folders() {
            fs::create_dir_all(&folder).map_err(not_writable(&folder))?;
        }
        let probe = self.root.join(".write-check");
        fs::write(&probe, b"").map_err(not_writable(&self.root))?;
        fs::remove_file(&probe).map_err(not_writable(&self.root))?;
        Ok(())
    }

    pub fn folders(&self) -> [PathBuf; 5] {
        [
            self.journal(),
            self.backups(),
            self.wallpapers(),
            self.themes(),
            self.logs(),
        ]
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ledger(&self) -> PathBuf {
        self.root.join("ledger.json")
    }

    pub fn history(&self) -> PathBuf {
        self.root.join("history.json")
    }

    pub fn config(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn journal(&self) -> PathBuf {
        self.root.join("journal")
    }

    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub fn wallpapers(&self) -> PathBuf {
        self.root.join("wallpapers")
    }

    pub fn themes(&self) -> PathBuf {
        self.root.join("themes")
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(path: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn resolves_tintcast_under_local_app_data_with_every_entry() {
        let base = tempfile::tempdir().unwrap();
        let dir = DataDir::open_in(Some(base.path().into())).unwrap();
        let root = base.path().join("Tintcast");
        assert_eq!(dir.root(), root);
        assert_eq!(dir.ledger(), root.join("ledger.json"));
        assert_eq!(dir.history(), root.join("history.json"));
        assert_eq!(dir.config(), root.join("config.toml"));
        assert_eq!(dir.journal(), root.join("journal"));
        assert_eq!(dir.backups(), root.join("backups"));
        assert_eq!(dir.wallpapers(), root.join("wallpapers"));
        assert_eq!(dir.themes(), root.join("themes"));
        assert_eq!(dir.logs(), root.join("logs"));
        assert_eq!(listing(base.path()), ["Tintcast"]);
        assert_eq!(
            listing(&root),
            ["backups", "journal", "logs", "themes", "wallpapers"]
        );
    }

    #[test]
    fn an_unset_local_app_data_is_an_error_that_names_the_path() {
        let error = DataDir::open_in(None).unwrap_err();
        assert!(matches!(error, DataDirError::LocalAppDataUnset));
        assert!(error.to_string().contains(r"%LOCALAPPDATA%\Tintcast"));
    }

    #[test]
    fn an_empty_or_relative_local_app_data_writes_nothing_in_the_current_folder() {
        for value in ["", "relative"] {
            let error = DataDir::open_in(Some(value.into())).unwrap_err();
            assert!(matches!(error, DataDirError::LocalAppDataUnset));
        }
        assert!(!Path::new("Tintcast").exists());
        assert!(!Path::new("relative").exists());
    }

    #[test]
    fn a_local_app_data_that_is_not_writable_is_an_error_that_names_the_path() {
        let base = tempfile::tempdir().unwrap();
        let blocker = base.path().join("not-a-folder");
        fs::write(&blocker, b"file").unwrap();
        let error = DataDir::open_in(Some(blocker.clone().into())).unwrap_err();
        let expected = blocker.join("Tintcast");
        match &error {
            DataDirError::NotWritable { path, .. } => assert_eq!(path, &expected),
            other => panic!("unexpected error {other:?}"),
        }
        assert!(error.to_string().contains(&expected.display().to_string()));
        assert_eq!(listing(base.path()), ["not-a-folder"]);
        assert_eq!(fs::read(&blocker).unwrap(), b"file");
    }
}
