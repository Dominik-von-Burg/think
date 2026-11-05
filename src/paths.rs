//! Where the notebook's files live.

use std::path::PathBuf;

pub(crate) struct Paths {
    pub(crate) data_dir: PathBuf,
    pub(crate) notebook: PathBuf,
    pub(crate) settings: PathBuf,
    pub(crate) repository: PathBuf,
}

impl Paths {
    pub(crate) fn new() -> Self {
        // Keep the notebook and its clone together, outside the installation directory.
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("think");
        Self {
            notebook: data_dir.join("notebook.txt"),
            settings: data_dir.join("settings.toml"),
            repository: data_dir.join("repository"),
            data_dir,
        }
    }
}
