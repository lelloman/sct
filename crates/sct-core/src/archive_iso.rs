//! Bounded local ISO writer. Catalog fencing/publication belongs to archive_export.
use crate::error::{Error, Result, require};
use sct_protocol::*;
use sha2::{Digest, Sha256};
use simple_server::io::{AsyncReadExt, AsyncWriteExt};
use std::{
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

fn io(_: std::io::Error) -> Error {
    Error(ErrorCode::DestinationUnavailable)
}
pub(crate) async fn private_directory(path: &Path) -> Result<()> {
    simple_server::fs::create_dir(path).await.map_err(io)?;
    simple_server::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(io)
}
pub(crate) fn regular(path: &Path) -> Result<simple_server::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(
            rustix::fs::OFlags::NOFOLLOW.bits() as i32 | rustix::fs::OFlags::NONBLOCK.bits() as i32,
        )
        .open(path)
        .map_err(io)?;
    require(
        file.metadata().map_err(io)?.is_file(),
        ErrorCode::IntegrityFailure,
    )?;
    Ok(simple_server::fs::File::from_std(file))
}
pub(crate) async fn new_file(path: &Path) -> Result<simple_server::fs::File> {
    simple_server::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .await
        .map_err(io)
}
pub(crate) async fn file_hash(path: &Path, maximum: u64) -> Result<(ChunkDigest, u64)> {
    let mut file = regular(path)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).await.map_err(io)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        require(size <= maximum, ErrorCode::IntegrityFailure)?;
        hash.update(&buffer[..n]);
    }
    Ok((format!("{:x}", hash.finalize()).parse()?, size))
}
pub(crate) async fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = new_file(path).await?;
    file.write_all(bytes).await.map_err(io)?;
    file.sync_all().await.map_err(io)
}
#[derive(Clone)]
pub struct InstalledArchiveBundle {
    pub manifest: ArchiveBundle,
    pub directory: PathBuf,
}
impl InstalledArchiveBundle {
    pub async fn open(directory: &Path) -> Result<Self> {
        let directory = simple_server::fs::canonicalize(directory)
            .await
            .map_err(io)?;
        let mut input = regular(&directory.join("recovery/bundle.json"))?;
        let mut bytes = Vec::new();
        (&mut input)
            .take(ARCHIVE_RECORD_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(io)?;
        require(
            bytes.len() <= ARCHIVE_RECORD_BYTES,
            ErrorCode::IntegrityFailure,
        )?;
        let manifest: ArchiveBundle = parse_canonical(&bytes)?;
        manifest.validate()?;
        require(
            manifest.platform == "linux-x86_64-glibc236",
            ErrorCode::InvalidRequest,
        )?;
        for required in [
            "START-HERE.txt",
            "recovery/sct-recover",
            "recovery/source.tar.gz",
        ] {
            require(
                manifest.files.iter().any(|f| f.path == required),
                ErrorCode::IntegrityFailure,
            )?;
        }
        let bundle = Self {
            manifest,
            directory,
        };
        for file in &bundle.manifest.files {
            bundle.checked_path(&file.path).await?;
            let (hash, size) =
                file_hash(&bundle.directory.join(&file.path), file.length.get()).await?;
            require(
                hash == file.sha256 && size == file.length.get(),
                ErrorCode::IntegrityFailure,
            )?;
        }
        Ok(bundle)
    }
    async fn checked_path(&self, path: &str) -> Result<PathBuf> {
        let mut current = self.directory.clone();
        for component in Path::new(path).components() {
            let std::path::Component::Normal(component) = component else {
                return Err(Error(ErrorCode::InvalidPath));
            };
            current.push(component);
            let metadata = simple_server::fs::symlink_metadata(&current)
                .await
                .map_err(io)?;
            require(
                !metadata.file_type().is_symlink(),
                ErrorCode::IntegrityFailure,
            )?;
        }
        Ok(current)
    }
    pub(crate) async fn copy_to(&self, destination: &Path) -> Result<()> {
        private_directory(&destination.join("recovery")).await?;
        write_file(
            &destination.join("recovery/bundle.json"),
            &canonical(&self.manifest)?,
        )
        .await?;
        for expected in &self.manifest.files {
            let path = self.checked_path(&expected.path).await?;
            let output = destination.join(&expected.path);
            if let Some(parent) = output.parent() {
                simple_server::fs::create_dir_all(parent)
                    .await
                    .map_err(io)?;
            }
            let mut source = regular(&path)?;
            let mut target = new_file(&output).await?;
            let mut hash = Sha256::new();
            let mut size = 0;
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let n = source.read(&mut buffer).await.map_err(io)?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                require(size <= expected.length.get(), ErrorCode::IntegrityFailure)?;
                hash.update(&buffer[..n]);
                target.write_all(&buffer[..n]).await.map_err(io)?;
            }
            require(
                size == expected.length.get()
                    && format!("{:x}", hash.finalize()) == expected.sha256.as_str(),
                ErrorCode::IntegrityFailure,
            )?;
            if expected.path == "recovery/sct-recover" {
                target
                    .set_permissions(std::fs::Permissions::from_mode(0o755))
                    .await
                    .map_err(io)?;
            }
            target.sync_all().await.map_err(io)?;
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct IsoWriter {
    pub executable: PathBuf,
    pub prlimit: PathBuf,
    pub parent_guard: PathBuf,
    pub(crate) inherited_lock: Option<std::sync::Arc<std::fs::File>>,
    pub timeout: Duration,
}
impl Default for IsoWriter {
    fn default() -> Self {
        Self {
            executable: "/usr/bin/xorriso".into(),
            prlimit: "/usr/bin/prlimit".into(),
            parent_guard: "/usr/bin/setpriv".into(),
            inherited_lock: None,
            timeout: Duration::from_secs(7200),
        }
    }
}
impl IsoWriter {
    /// Read at most 8 KiB of version output; no unbounded child output is buffered.
    pub async fn version(&self) -> Result<String> {
        let mut child = simple_server::process::ManagedCommand::new(&self.executable)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(io)?;
        let mut output = Vec::new();
        let mut stdout = child.stdout.take().ok_or(Error(ErrorCode::InternalError))?;
        simple_server::time::timeout(Duration::from_secs(10), async {
            (&mut stdout)
                .take(8193)
                .read_to_end(&mut output)
                .await
                .map_err(io)?;
            require(output.len() <= 8192, ErrorCode::DestinationUnavailable)?;
            require(
                child.wait().await.map_err(io)?.success(),
                ErrorCode::DestinationUnavailable,
            )
        })
        .await
        .map_err(|_| Error(ErrorCode::DestinationUnavailable))??;
        String::from_utf8(output).map_err(|_| Error(ErrorCode::IntegrityFailure))
    }
    async fn run(&self, args: &[std::ffi::OsString], limit: u64) -> Result<()> {
        let stdin = match &self.inherited_lock {
            Some(file) => Stdio::from(file.try_clone().map_err(io)?),
            None => Stdio::null(),
        };
        let mut child = simple_server::process::ManagedCommand::new(&self.parent_guard)
            .args(["--pdeathsig", "KILL", "--"])
            .arg(&self.prlimit)
            .arg(format!("--fsize={limit}:{limit}"))
            .arg("--")
            .arg(&self.executable)
            .args(args)
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(io)?;
        let status = simple_server::time::timeout(self.timeout, child.wait())
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))?
            .map_err(io)?;
        require(status.success(), ErrorCode::DestinationUnavailable)
    }
    pub async fn write(&self, tree: &Path, output: &Path, capacity: u64) -> Result<()> {
        require(
            !output.try_exists().map_err(io)?,
            ErrorCode::RevisionConflict,
        )?;
        let args: Vec<std::ffi::OsString> = vec![
            "-as".into(),
            "mkisofs".into(),
            "-quiet".into(),
            "-iso-level".into(),
            "3".into(),
            "-r".into(),
            "-uid".into(),
            "0".into(),
            "-gid".into(),
            "0".into(),
            "-V".into(),
            "SCT_ARCHIVE_V1".into(),
            "--modification-date=2026010100000000".into(),
            "-o".into(),
            output.as_os_str().to_owned(),
            tree.as_os_str().to_owned(),
        ];
        self.run(&args, capacity).await?;
        let mut file = regular(output)?;
        require(
            file.metadata().await.map_err(io)?.len() <= capacity,
            ErrorCode::ArchiveMetadataTooLarge,
        )?;
        file.sync_all().await.map_err(io)
    }
    pub async fn extract(&self, image: &Path, destination: &Path, capacity: u64) -> Result<()> {
        require(
            !destination.try_exists().map_err(io)?,
            ErrorCode::RevisionConflict,
        )?;
        self.run(
            &[
                "-abort_on".into(),
                "FAILURE".into(),
                "-osirrox".into(),
                "on".into(),
                "-indev".into(),
                image.as_os_str().to_owned(),
                "-extract".into(),
                "/".into(),
                destination.as_os_str().to_owned(),
            ],
            capacity,
        )
        .await
    }
}
