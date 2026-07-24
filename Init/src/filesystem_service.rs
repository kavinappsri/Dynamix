use crate::service::Service;
use std::fs::create_dir_all;

pub struct FilesystemService;

impl FilesystemService {
    const DIRECTORIES: [&str; 7] = [
        "../system",
        "../config",
        "../home",
        "../apps",
        "../runtime",
        "../logs",
        "../tmp",
    ];

    pub fn new() -> Self {
        Self
    }

    fn load_filesystem() -> Result<(), String>  {
        for directory in Self::DIRECTORIES {
            create_dir_all(directory).map_err(|e| format!("Failed to create {directory}: {e}"))?;
        }
        Ok(())
    }
}

impl Service for FilesystemService {
    fn name(&self) -> &str {
        "Filesystem Service"
    }

    fn start(&mut self) -> Result<(), String> {
        Self::load_filesystem()
    }

    fn stop(&self) {
        todo!()
    }
}