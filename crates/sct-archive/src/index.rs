use crate::*;
use sqlx::{
    ConnectOptions, Connection, Row, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader},
    os::unix::fs::OpenOptionsExt,
};

pub struct ArchiveIndex {
    _lock: std::fs::File,
    pub connection: SqliteConnection,
    pub descriptor: ArchiveDiscDescriptor,
}
async fn connect(path: &Path) -> Result<SqliteConnection> {
    Ok(SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Delete)
            .synchronous(SqliteSynchronous::Full)
            .statement_cache_capacity(32)
            .pragma("cache_size", "-8192")
            .pragma("temp_store", "FILE")
            .disable_statement_logging(),
    )
    .await?)
}
impl ArchiveIndex {
    /// `database` must be a new file in a private, caller-owned working directory.
    /// Invalid media never produces a committed/usable index.
    pub async fn create(media: &Path, database: &Path) -> Result<Self> {
        let dir = SafeDir::open(media)?;
        let desc = descriptor(&dir)?;
        verify_bundle(&dir, &desc)?;
        let lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(database)?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
        let mut connection = connect(database).await?;
        let mut tx = connection.begin().await?;
        sqlx::raw_sql("CREATE TABLE meta(descriptor TEXT NOT NULL,snapshot TEXT NOT NULL);
            CREATE TABLE discs(id TEXT PRIMARY KEY,number INTEGER NOT NULL UNIQUE,set_sequence INTEGER NOT NULL);
            CREATE TABLE entries(path TEXT PRIMARY KEY,kind TEXT NOT NULL,root TEXT,version TEXT,length INTEGER NOT NULL);
            CREATE TABLE objects(root TEXT,digest TEXT,length INTEGER NOT NULL,PRIMARY KEY(root,digest),UNIQUE(root,digest,length));
            CREATE TABLE extents(path TEXT REFERENCES entries(path),offset INTEGER,root TEXT,digest TEXT,length INTEGER,PRIMARY KEY(path,offset),FOREIGN KEY(root,digest,length) REFERENCES objects(root,digest,length));
            CREATE TABLE placements(root TEXT,digest TEXT,length INTEGER,disc TEXT REFERENCES discs(id),PRIMARY KEY(root,digest,disc),FOREIGN KEY(root,digest,length) REFERENCES objects(root,digest,length));
            CREATE INDEX placement_disc ON placements(disc,root,digest);")
            .execute(&mut *tx).await?;
        let mut reader = BufReader::new(dir.file("snapshot.jsonl")?);
        let mut hash = Sha256::new();
        let mut records = 0u64;
        let mut bytes = 0u64;
        let mut last = (0u8, String::new());
        let mut sources = Vec::new();
        let mut snapshot = String::new();
        let mut active_file: Option<(String, String, u64, u64)> = None;
        loop {
            let mut line = Vec::new();
            (&mut reader)
                .take(ARCHIVE_RECORD_BYTES as u64 + 2)
                .read_until(b'\n', &mut line)?;
            if line.is_empty() {
                break;
            }
            ensure(
                line.last() == Some(&b'\n') && line.len() <= ARCHIVE_RECORD_BYTES + 1,
                "oversized or unterminated index record",
            )?;
            bytes += line.len() as u64;
            records += 1;
            ensure(
                bytes <= desc.index_bytes && records <= desc.index_records,
                "index exceeds declared bounds",
            )?;
            hash.update(&line);
            let record = archive_record(&line[..line.len() - 1])?;
            if records == 1 {
                snapshot = String::from_utf8(line[..line.len() - 1].to_vec())?;
            }
            let order = match &record {
                ArchiveRecord::Snapshot { .. } => (0, String::new()),
                ArchiveRecord::Disc { number, .. } => (1, format!("{number:020}")),
                ArchiveRecord::Directory { path } | ArchiveRecord::File { path, .. } => {
                    (2, path.as_str().into())
                }
                ArchiveRecord::Extent { path, offset, .. } => {
                    (3, format!("{}\0{:020}", path.as_str(), offset.get()))
                }
                ArchiveRecord::Placement { object, disc_id } => (
                    4,
                    format!(
                        "{}\0{}\0{}",
                        object.root_id.as_str(),
                        object.digest.as_str(),
                        disc_id
                    ),
                ),
            };
            ensure(
                records == 1 || order > last,
                "duplicate or unordered archive records",
            )?;
            ensure(records != 1 || order.0 == 0, "missing snapshot header")?;
            last = order;
            match record {
                ArchiveRecord::Snapshot {
                    format: _,
                    collection_id,
                    set_id,
                    set_sequence,
                    first_disc,
                    last_disc,
                    sources: selected,
                } => {
                    ensure(
                        collection_id == desc.collection_id
                            && set_id == desc.set_id
                            && set_sequence == desc.set_sequence
                            && first_disc == desc.first_disc
                            && last_disc == desc.last_disc,
                        "descriptor and header disagree",
                    )?;
                    let mut unique = std::collections::BTreeSet::new();
                    for source in &selected {
                        ensure(
                            unique.insert((
                                source.root_id.as_str(),
                                source.version_id.as_str(),
                                source.path.as_str(),
                            )),
                            "duplicate source selection",
                        )?;
                    }
                    sources = selected;
                }
                ArchiveRecord::Disc {
                    disc_id,
                    number,
                    set_sequence,
                } => {
                    ensure(
                        (set_sequence < desc.set_sequence && number < desc.first_disc)
                            || (set_sequence == desc.set_sequence
                                && (desc.first_disc..=desc.last_disc).contains(&number)),
                        "future or inconsistent disc reference",
                    )?;
                    sqlx::query("INSERT INTO discs VALUES(?,?,?)")
                        .bind(disc_id)
                        .bind(number as i64)
                        .bind(set_sequence as i64)
                        .execute(&mut *tx)
                        .await?;
                }
                ArchiveRecord::Directory { path } => {
                    parent(&mut tx, &path).await?;
                    sqlx::query("INSERT INTO entries VALUES(?,'directory',NULL,NULL,0)")
                        .bind(path.as_str())
                        .execute(&mut *tx)
                        .await?;
                }
                ArchiveRecord::File {
                    path,
                    root_id,
                    version_id,
                    length,
                } => {
                    parent(&mut tx, &path).await?;
                    ensure(
                        sources.iter().any(|s| {
                            s.root_id == root_id
                                && s.version_id == version_id
                                && (s.path == path
                                    || path.as_str().starts_with(&format!(
                                        "{}/",
                                        s.path.as_str().trim_end_matches('/')
                                    )))
                        }),
                        "file outside source selection",
                    )?;
                    sqlx::query("INSERT INTO entries VALUES(?,'file',?,?,?)")
                        .bind(path.as_str())
                        .bind(root_id.as_str())
                        .bind(version_id.as_str())
                        .bind(length.get() as i64)
                        .execute(&mut *tx)
                        .await?;
                }
                ArchiveRecord::Extent {
                    path,
                    offset,
                    object,
                } => {
                    if active_file.as_ref().is_none_or(|f| f.0 != path.as_str()) {
                        if let Some((_, _, expected, actual)) = active_file.take() {
                            ensure(expected == actual, "incomplete file extent coverage")?;
                        }
                        let file = sqlx::query(
                            "SELECT root,length FROM entries WHERE path=? AND kind='file'",
                        )
                        .bind(path.as_str())
                        .fetch_optional(&mut *tx)
                        .await?
                        .ok_or("extent has no file")?;
                        active_file = Some((
                            path.as_str().into(),
                            file.get(0),
                            file.get::<i64, _>(1) as u64,
                            0,
                        ));
                    }
                    let (_, root, length, end) = active_file.as_mut().ok_or("missing file")?;
                    ensure(
                        root == object.root_id.as_str() && *end == offset.get(),
                        "extent root mismatch or gap",
                    )?;
                    *end += object.length.get();
                    ensure(*end <= *length, "extent exceeds file length")?;
                    sqlx::query(
                        "INSERT INTO objects VALUES(?,?,?) ON CONFLICT(root,digest) DO NOTHING",
                    )
                    .bind(object.root_id.as_str())
                    .bind(object.digest.as_str())
                    .bind(object.length.get() as i64)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query("INSERT INTO extents VALUES(?,?,?,?,?)")
                        .bind(path.as_str())
                        .bind(offset.get() as i64)
                        .bind(object.root_id.as_str())
                        .bind(object.digest.as_str())
                        .bind(object.length.get() as i64)
                        .execute(&mut *tx)
                        .await?;
                }
                ArchiveRecord::Placement { object, disc_id } => {
                    sqlx::query("INSERT INTO placements VALUES(?,?,?,?)")
                        .bind(object.root_id.as_str())
                        .bind(object.digest.as_str())
                        .bind(object.length.get() as i64)
                        .bind(disc_id)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
        if let Some((_, _, expected, actual)) = active_file {
            ensure(expected == actual, "incomplete file extent coverage")?;
        }
        ensure(
            bytes == desc.index_bytes
                && records == desc.index_records
                && format!("{:x}", hash.finalize()) == desc.index_sha256.as_str(),
            "index checksum or size mismatch",
        )?;
        let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entries e WHERE kind='file' AND length>0 AND NOT EXISTS(SELECT 1 FROM extents x WHERE x.path=e.path)) OR EXISTS(SELECT 1 FROM objects o WHERE NOT EXISTS(SELECT 1 FROM placements p WHERE p.root=o.root AND p.digest=o.digest)) OR NOT EXISTS(SELECT 1 FROM entries WHERE path='/' AND kind='directory')").fetch_one(&mut *tx).await?;
        ensure(!invalid, "incomplete snapshot graph")?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM discs WHERE set_sequence=?")
            .bind(desc.set_sequence as i64)
            .fetch_one(&mut *tx)
            .await?;
        ensure(
            count as u64 == desc.last_disc - desc.first_disc + 1,
            "missing current-set disc",
        )?;
        let identity: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM discs WHERE id=? AND number=? AND set_sequence=?)",
        )
        .bind(&desc.disc_id)
        .bind(desc.number as i64)
        .bind(desc.set_sequence as i64)
        .fetch_one(&mut *tx)
        .await?;
        ensure(identity, "descriptor disc absent from index")?;
        sqlx::query("INSERT INTO meta VALUES(?,?)")
            .bind(serde_json::to_string(&desc)?)
            .bind(snapshot)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Self {
            _lock: lock,
            connection,
            descriptor: desc,
        })
    }
    pub async fn open(database: &Path) -> Result<Self> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(database)?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
        let mut connection = connect(database).await?;
        let value: String = sqlx::query_scalar("SELECT descriptor FROM meta")
            .fetch_one(&mut connection)
            .await?;
        let descriptor: ArchiveDiscDescriptor = serde_json::from_str(&value)?;
        descriptor.validate()?;
        Ok(Self {
            _lock: lock,
            connection,
            descriptor,
        })
    }
    pub async fn inspect(&mut self, output: &mut impl Write) -> Result<()> {
        serde_json::to_writer(&mut *output, &self.descriptor)?;
        output.write_all(b"\n")?;
        let mut number = 0i64;
        loop {
            let rows = sqlx::query(
                "SELECT id,number,set_sequence FROM discs WHERE number>? ORDER BY number LIMIT 128",
            )
            .bind(number)
            .fetch_all(&mut self.connection)
            .await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                number = row.get(1);
                serde_json::to_writer(
                    &mut *output,
                    &serde_json::json!({"kind":"disc","disc_id":row.get::<String,_>(0),"number":number,"set_sequence":row.get::<i64,_>(2)}),
                )?;
                output.write_all(b"\n")?;
            }
        }
        let mut after = String::new();
        loop {
            let rows = sqlx::query(
                "SELECT path,kind,length FROM entries WHERE path>? ORDER BY path LIMIT 128",
            )
            .bind(&after)
            .fetch_all(&mut self.connection)
            .await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                after = row.get(0);
                serde_json::to_writer(
                    &mut *output,
                    &serde_json::json!({"path":after,"kind":row.get::<String,_>(1),"length":row.get::<i64,_>(2)}),
                )?;
                output.write_all(b"\n")?;
            }
        }
        Ok(())
    }
}
async fn parent(db: &mut SqliteConnection, path: &RootPath) -> Result<()> {
    if path.as_str() == "/" {
        return Ok(());
    }
    let (prefix, _) = path.as_str().rsplit_once('/').ok_or("invalid entry path")?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM entries WHERE path=? AND kind='directory')",
    )
    .bind(if prefix.is_empty() { "/" } else { prefix })
    .fetch_one(db)
    .await?;
    ensure(exists, "missing or non-directory parent")
}
