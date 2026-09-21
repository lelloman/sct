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

impl SafeDir {
    pub fn directory(&self, relative: &str) -> Result<Self> {
        if relative.is_empty() {
            return Ok(Self(self.0.try_clone()?));
        }
        let path: sct_protocol::RootPath = format!("/{relative}").parse()?;
        sct_protocol::archive_path(&path)?;
        let mut dir = self.0.try_clone()?;
        for part in relative.split('/') {
            match rustix::fs::mkdirat(&dir, part, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(e.into()),
            }
            dir = openat(
                &dir,
                part,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?
            .into();
        }
        Ok(Self(dir))
    }
    /// Only use in a private disposable cache, never in a restore destination.
    pub fn remove_cached_file(&self, relative: &str) -> Result<()> {
        match self.file(relative) {
            Ok(_) => {}
            Err(e) if e.downcast_ref::<rustix::io::Errno>() == Some(&rustix::io::Errno::NOENT) => {
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
        let dir = self.directory(parent)?;
        rustix::fs::unlinkat(&dir.0, name, rustix::fs::AtFlags::empty())?;
        dir.0.sync_all()?;
        Ok(())
    }
    pub fn pending(&self, relative: &str) -> Result<PendingFile> {
        let path: sct_protocol::RootPath = format!("/{relative}").parse()?;
        sct_protocol::archive_path(&path)?;
        let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
        let dir = self.directory(parent)?;
        let temporary = format!(".sct-recovery-{}", uuid::Uuid::new_v4());
        let file = openat(
            &dir.0,
            &temporary,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?
        .into();
        Ok(PendingFile {
            dir,
            temporary,
            name: name.into(),
            file,
        })
    }
}
pub struct PendingFile {
    dir: SafeDir,
    temporary: String,
    name: String,
    pub file: File,
}
impl PendingFile {
    pub fn publish(self) -> Result<()> {
        self.file.sync_all()?;
        // Hard-link publication is atomic and refuses any existing destination.
        rustix::fs::linkat(
            &self.dir.0,
            &self.temporary,
            &self.dir.0,
            &self.name,
            rustix::fs::AtFlags::empty(),
        )?;
        self.dir.0.sync_all()?;
        Ok(())
    }
}
impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = rustix::fs::unlinkat(&self.dir.0, &self.temporary, rustix::fs::AtFlags::empty());
    }
}
