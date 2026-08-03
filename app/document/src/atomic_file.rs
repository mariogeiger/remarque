use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

pub fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_file_atomically(path, |file| file.write_all(bytes))
}

pub(crate) fn write_file_atomically(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let (temporary_path, mut file) = create_temporary_file(parent, path.file_name())?;
    let mut temporary = TemporaryPath::new(temporary_path);
    write(&mut file)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary.path(), path)?;
    temporary.keep();
    File::open(parent)?.sync_all()
}

fn create_temporary_file(
    parent: &Path,
    destination_name: Option<&std::ffi::OsStr>,
) -> io::Result<(PathBuf, File)> {
    let destination_name = destination_name.unwrap_or_default().to_string_lossy();
    loop {
        let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{destination_name}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

struct TemporaryPath {
    path: PathBuf,
    remove_on_drop: bool,
}

impl TemporaryPath {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            remove_on_drop: true,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn keep(&mut self) {
        self.remove_on_drop = false;
    }
}

impl Drop for TemporaryPath {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remarque-atomic-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn replaces_the_destination_and_leaves_no_temporary_file() {
        let path = test_path("replace");
        write_bytes_atomically(&path, b"first").unwrap();
        write_bytes_atomically(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let prefix = format!(
            ".{}.{}.",
            path.file_name().unwrap().to_string_lossy(),
            std::process::id()
        );
        assert!(
            fs::read_dir(path.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().starts_with(&prefix))
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn removes_the_temporary_file_when_writing_fails() {
        let path = test_path("failure");
        let error = write_file_atomically(&path, |_| Err(io::Error::other("stopped"))).unwrap_err();
        assert_eq!(error.to_string(), "stopped");
        assert!(!path.exists());
    }
}
