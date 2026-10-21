use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

// Prepare a complete replacement before renaming. Never truncate the installed
// file: a locked DLL/EXE must fail while preserving its previous bytes.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    reject_link(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("No parent directory"))?;
    let temp = parent.join(format!(
        ".ascii-cam-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn reject_link(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(io::Error::other("Reparse points are not supported"));
                }
            }
            if metadata.file_type().is_symlink() {
                return Err(io::Error::other("Symbolic links are not supported"));
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn snapshot(path: &Path) -> io::Result<Option<Vec<u8>>> {
    reject_link(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Default)]
pub struct FileTransaction {
    changes: Vec<(PathBuf, Option<Vec<u8>>)>,
    finished: bool,
}
impl FileTransaction {
    pub fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let before = snapshot(path)?;
        atomic_write(path, bytes)?;
        self.changes.push((path.to_owned(), before));
        Ok(())
    }
    pub fn remove(&mut self, path: &Path) -> io::Result<()> {
        let before = snapshot(path)?;
        if before.is_some() {
            fs::remove_file(path)?;
            self.changes.push((path.to_owned(), before));
        }
        Ok(())
    }
    pub fn commit(&mut self) {
        self.finished = true;
        self.changes.clear();
    }
    pub fn rollback(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for (path, before) in self.changes.drain(..).rev() {
            let result = if let Some(bytes) = before {
                atomic_write(&path, &bytes)
            } else {
                match fs::remove_file(&path) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                    r => r,
                }
            };
            if let Err(error) = result {
                errors.push(format!("Could not restore {}: {error}", path.display()));
            }
        }
        self.finished = true;
        errors
    }
}
impl Drop for FileTransaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.rollback();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "ascii-setup-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn install_failure_restores_old_files_and_removes_new_ones() {
        let dir = directory();
        let old = dir.join("old.dll");
        let new = dir.join("new.exe");
        let unrelated = dir.join("notes.txt");
        fs::write(&old, b"original").unwrap();
        fs::write(&unrelated, b"keep").unwrap();
        let mut tx = FileTransaction::default();
        tx.write(&old, b"replacement").unwrap();
        tx.write(&new, b"new").unwrap();
        assert!(tx.write(&dir.join("missing/file"), b"fail").is_err());
        assert!(tx.rollback().is_empty());
        assert_eq!(fs::read(&old).unwrap(), b"original");
        assert!(!new.exists());
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn uninstall_failure_restores_deleted_files() {
        let dir = directory();
        let p = dir.join("client.exe");
        fs::write(&p, b"client").unwrap();
        {
            let mut tx = FileTransaction::default();
            tx.remove(&p).unwrap();
            assert!(!p.exists());
        }
        assert_eq!(fs::read(&p).unwrap(), b"client");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn committed_install_keeps_files_and_repeated_edits_roll_back_in_order() {
        let dir = directory();
        let p = dir.join("client.exe");
        {
            let mut tx = FileTransaction::default();
            tx.write(&p, b"v1").unwrap();
            tx.commit();
        }
        {
            let mut tx = FileTransaction::default();
            tx.write(&p, b"v2").unwrap();
            tx.write(&p, b"v3").unwrap();
        }
        assert_eq!(fs::read(&p).unwrap(), b"v1");
        fs::remove_dir_all(dir).unwrap();
    }
}
