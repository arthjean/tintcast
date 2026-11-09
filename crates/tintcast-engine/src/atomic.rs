use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[must_use = "a staged write changes nothing until it is committed"]
#[derive(Debug)]
pub struct StagedWrite {
    temp: PathBuf,
    target: PathBuf,
}

pub fn stage(target: &Path, bytes: &[u8]) -> io::Result<StagedWrite> {
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no folder"))?;
    fs::create_dir_all(parent)?;
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    let temp = parent.join(format!(
        ".{}.{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = File::create(&temp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(StagedWrite {
        temp,
        target: target.to_path_buf(),
    })
}

impl StagedWrite {
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    pub fn commit(self) -> io::Result<()> {
        let result = fs::rename(&self.temp, &self.target);
        if result.is_err() {
            let _ = fs::remove_file(&self.temp);
        }
        result
    }

    pub fn abandon(self) {
        let _ = fs::remove_file(&self.temp);
    }
}

pub fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    stage(target, bytes)?.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_replaces_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("ledger.json");
        fs::write(&target, b"old").unwrap();
        write_atomic(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_write_stopped_before_the_rename_leaves_the_previous_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("history.json");
        fs::write(&target, b"previous").unwrap();
        let staged = stage(&target, b"half done").unwrap();
        assert_eq!(fs::read(staged.temp_path()).unwrap(), b"half done");
        std::mem::forget(staged);
        assert_eq!(fs::read(&target).unwrap(), b"previous");
    }

    #[test]
    fn creates_missing_folders() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a").join("b").join("file.json");
        write_atomic(&target, b"{}").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"{}");
    }
}
