use std::{fs, path::PathBuf};

use anyhow::Result;

#[derive(Clone, Debug)]
pub struct FileSystemDocumentSource {
    path: PathBuf,
}

impl FileSystemDocumentSource {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn read_to_string(&self) -> Result<String> {
        Ok(fs::read_to_string(&self.path)?)
    }
}
