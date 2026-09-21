//! Descriptor-relative media access. Never follow untrusted media symlinks.
use crate::{Result, ensure};
use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
use std::{fs::File, path::Path};

pub struct SafeDir(File);
impl SafeDir {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self(
            open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?
            .into(),
        ))
    }
    pub fn file(&self, relative: &str) -> Result<File> {
        let path: sct_protocol::RootPath = format!("/{relative}").parse()?;
        sct_protocol::archive_path(&path)?;
        ensure(!relative.starts_with('/'), "absolute media path")?;
        let parts: Vec<_> = relative.split('/').collect();
        let mut dir = self.0.try_clone()?;
        for part in &parts[..parts.len() - 1] {
            dir = openat(
                &dir,
                *part,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?
            .into();
        }
        let file: File = openat(
            &dir,
            parts[parts.len() - 1],
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?
        .into();
        ensure(
            FileType::from_raw_mode(fstat(&file)?.st_mode) == FileType::RegularFile,
            "non-regular archive file",
        )?;
        Ok(file)
    }
}
