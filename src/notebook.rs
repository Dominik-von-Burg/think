//! File ownership and atomic saves for the desktop notebook.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
};

use tempfile::NamedTempFile;

pub(crate) fn lock(data_dir: &Path) -> io::Result<File> {
    fs::create_dir_all(data_dir)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(data_dir.join(".notebook.lock"))?;
    lock.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "Think already has this notebook open",
        ),
        fs::TryLockError::Error(error) => error,
    })?;
    Ok(lock)
}

pub(crate) fn read(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        result => result,
    }
}

pub(crate) fn save(path: &Path, text: &str) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;
    let mut staged = NamedTempFile::new_in(directory)?;
    staged.write_all(text.as_bytes())?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_desktop_can_own_the_notebook() {
        let directory = tempfile::tempdir().unwrap();
        let first = lock(directory.path()).unwrap();
        assert_eq!(
            lock(directory.path()).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(first);
        assert!(lock(directory.path()).is_ok());
    }
}
