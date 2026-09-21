//! Operator-controlled verification and isolated service recovery.
use crate::{
    checkpoint_export::{CatalogHeader, CatalogRecord, identifier},
    error::{Error, Result, require},
    local_store::LocalStore,
    payload::{ObjectKey, ObjectStore},
};
use sct_protocol::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
fn io(_: std::io::Error) -> Error {
    Error(ErrorCode::DestinationUnavailable)
}
async fn private_directory(path: &Path) -> Result<()> {
    tokio::fs::create_dir(path).await.map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(io)?;
    }
    Ok(())
}
const MAX_LINE: usize = 16 * 1024 * 1024;
async fn line(reader: &mut BufReader<tokio::fs::File>) -> Result<Option<Vec<u8>>> {
    let mut result = Vec::new();
    loop {
        let bytes = reader.fill_buf().await.map_err(io)?;
        if bytes.is_empty() {
            return if result.is_empty() {
                Ok(None)
            } else {
                Err(Error(ErrorCode::IntegrityFailure))
            };
        }
        let end = bytes.iter().position(|c| *c == b'\n').map(|n| n + 1);
        let size = end.unwrap_or(bytes.len());
        require(result.len() + size <= MAX_LINE, ErrorCode::IntegrityFailure)?;
        result.extend_from_slice(&bytes[..size]);
        reader.consume(size);
        if end.is_some() {
            return Ok(Some(result));
        }
    }
}
async fn hash_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await.map_err(io)?;
    let mut hash = Sha256::new();
    let mut bytes = vec![0u8; 65536];
    loop {
        let n = file.read(&mut bytes).await.map_err(io)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn key(object: &CheckpointObject) -> Result<ObjectKey> {
    Ok(ObjectKey {
        root: object.root_id.clone(),
        digest: object.digest.clone(),
        generation: object.generation.parse()?,
    })
}
async fn checked_path(base: &Path, object: &CheckpointObject) -> Result<PathBuf> {
    let candidate = base
        .join(object.backend_id.as_str())
        .join(key(object)?.path());
    let canonical = tokio::fs::canonicalize(&candidate).await.map_err(io)?;
    // Independent packages contain actual files, never links into production.
    require(
        canonical == candidate && canonical.starts_with(base),
        ErrorCode::InvalidRequest,
    )?;
    let metadata = tokio::fs::symlink_metadata(&candidate).await.map_err(io)?;
    require(
        metadata.is_file() && metadata.len() == object.bytes.get(),
        ErrorCode::IntegrityFailure,
    )?;
    Ok(candidate)
}
async fn inspect_package(package: &Path) -> Result<CheckpointDescriptor> {
    let mut descriptor_file = BufReader::new(
        tokio::fs::File::open(package.join("descriptor.json"))
            .await
            .map_err(io)?,
    );
    let descriptor: CheckpointDescriptor = serde_json::from_slice(
        &line(&mut descriptor_file)
            .await?
            .ok_or(Error(ErrorCode::IntegrityFailure))?,
    )?;
    require(
        line(&mut descriptor_file).await?.is_none(),
        ErrorCode::IntegrityFailure,
    )?;
    descriptor.validate()?;
    require(
        hash_file(&package.join("catalog.jsonl")).await? == descriptor.catalog_digest.as_str(),
        ErrorCode::IntegrityFailure,
    )?;
    require(
        hash_file(&package.join("inventory.jsonl")).await? == descriptor.inventory_digest.as_str(),
        ErrorCode::IntegrityFailure,
    )?;
    let mut catalog = BufReader::new(
        tokio::fs::File::open(package.join("catalog.jsonl"))
            .await
            .map_err(io)?,
    );
    let header: CatalogHeader = serde_json::from_slice(
        &line(&mut catalog)
            .await?
            .ok_or(Error(ErrorCode::IntegrityFailure))?,
    )?;
    require(
        header.format == 1
            && header.schema_version == 22
            && header.checkpoint_id == descriptor.checkpoint_id
            && header.generation_watermark == descriptor.generation_watermark,
        ErrorCode::IntegrityFailure,
    )?;
    Ok(descriptor)
}
async fn independent_directory(package: &Path, payloads: &Path) -> Result<()> {
    let mut catalog = BufReader::new(
        tokio::fs::File::open(package.join("catalog.jsonl"))
            .await
            .map_err(io)?,
    );
    let header: CatalogHeader = serde_json::from_slice(
        &line(&mut catalog)
            .await?
            .ok_or(Error(ErrorCode::IntegrityFailure))?,
    )?;
    while let Some(bytes) = line(&mut catalog).await? {
        let record: CatalogRecord = serde_json::from_slice(&bytes)?;
        require(
            header.tables.contains(&record.table) && record.row.is_object(),
            ErrorCode::IntegrityFailure,
        )?;
        if record.table == "backends"
            && let Some(source) = record.row["destination_identity"]
                .as_str()
                .and_then(|s| s.strip_prefix("local:"))
        {
            let source = Path::new(source);
            require(
                !payloads.starts_with(source) && !source.starts_with(payloads),
                ErrorCode::InvalidRequest,
            )?;
        }
    }
    Ok(())
}
pub async fn verify_checkpoint_package(
    package: &Path,
    payloads: &Path,
) -> Result<CheckpointDescriptor> {
    let descriptor = inspect_package(package).await?;
    let payloads = tokio::fs::canonicalize(payloads).await.map_err(io)?;
    independent_directory(package, &payloads).await?;
    let mut inventory = BufReader::new(
        tokio::fs::File::open(package.join("inventory.jsonl"))
            .await
            .map_err(io)?,
    );
    while let Some(bytes) = line(&mut inventory).await? {
        let object: CheckpointObject = serde_json::from_slice(&bytes)?;
        let path = checked_path(&payloads, &object).await?;
        require(
            hash_file(&path).await? == object.digest.as_str(),
            ErrorCode::IntegrityFailure,
        )?;
    }
    Ok(CheckpointDescriptor {
        package_state: RecoveryPackageState::PayloadVerified,
        ..descriptor
    })
}

/// Explicit operator copying; sources are supplied by the caller, never inferred
/// from the old catalog. The destination must be a fresh independent directory.
pub async fn copy_checkpoint_payloads(
    package: &Path,
    sources: &crate::payload::Stores,
    destination: &Path,
) -> Result<CheckpointDescriptor> {
    inspect_package(package).await?;
    let destination = tokio::fs::canonicalize(destination).await.map_err(io)?;
    independent_directory(package, &destination).await?;
    require(
        tokio::fs::read_dir(&destination)
            .await
            .map_err(io)?
            .next_entry()
            .await
            .map_err(io)?
            .is_none(),
        ErrorCode::InvalidRequest,
    )?;
    let mut targets = std::collections::BTreeMap::new();
    let mut inventory = BufReader::new(
        tokio::fs::File::open(package.join("inventory.jsonl"))
            .await
            .map_err(io)?,
    );
    while let Some(bytes) = line(&mut inventory).await? {
        let object: CheckpointObject = serde_json::from_slice(&bytes)?;
        let backend = object.backend_id.as_str();
        let source = sources
            .get(backend)
            .ok_or(Error(ErrorCode::DestinationUnavailable))?;
        if !targets.contains_key(backend) {
            let path = destination.join(backend);
            private_directory(&path).await?;
            targets.insert(backend.to_owned(), LocalStore::new(&path, 0)?);
        }
        let key = key(&object)?;
        targets[backend]
            .put(&key, object.bytes.get(), source.read(&key).await?)
            .await?;
    }
    verify_checkpoint_package(package, &destination).await
}

async fn header(package: &Path) -> Result<CatalogHeader> {
    let mut reader = BufReader::new(
        tokio::fs::File::open(package.join("catalog.jsonl"))
            .await
            .map_err(io)?,
    );
    Ok(serde_json::from_slice(
        &line(&mut reader)
            .await?
            .ok_or(Error(ErrorCode::IntegrityFailure))?,
    )?)
}
async fn flush_rows(tx: &mut sqlx::PgConnection, rows: &mut Vec<CatalogRecord>) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO sct_recovery_rows(table_name,payload) ",
    );
    query.push_values(rows.iter(), |mut b, r| {
        b.push_bind(&r.table).push_bind(&r.row);
    });
    query.build().execute(tx).await?;
    rows.clear();
    Ok(())
}
impl crate::catalog::Catalog {
    /// Requires an explicitly selected isolated catalog and an empty destination.
    /// Never opens any destination or credential reference from the old catalog.
    pub async fn recover_checkpoint(
        &self,
        package: &Path,
        payloads: &Path,
        destination: &Path,
        acknowledge_isolated: bool,
    ) -> Result<CheckpointDescriptor> {
        use sqlx::Row;
        use std::collections::{BTreeMap, BTreeSet};
        require(acknowledge_isolated, ErrorCode::InvalidRequest)?;
        let descriptor = verify_checkpoint_package(package, payloads).await?;
        let header = header(package).await?;
        require(
            header.format == 1 && header.schema_version == 22,
            ErrorCode::UnsupportedOperation,
        )?;
        let payloads = tokio::fs::canonicalize(payloads).await.map_err(io)?;
        let destination = tokio::fs::canonicalize(destination).await.map_err(io)?;
        require(
            !destination.starts_with(&payloads) && !payloads.starts_with(&destination),
            ErrorCode::InvalidRequest,
        )?;
        require(
            tokio::fs::read_dir(&destination)
                .await
                .map_err(io)?
                .next_entry()
                .await
                .map_err(io)?
                .is_none(),
            ErrorCode::InvalidRequest,
        )?;
        // Inspect before migrations: arbitrary existing application tables are
        // never repurposed, even if a caller points at the wrong database.
        let existing: Vec<String> = sqlx::query_scalar(
            "SELECT tablename FROM pg_tables WHERE schemaname=current_schema() ORDER BY tablename",
        )
        .fetch_all(&self.pool)
        .await?;
        for table in &existing {
            require(
                table == "_sqlx_migrations" || header.tables.contains(table),
                ErrorCode::InvalidRequest,
            )?;
            if table != "_sqlx_migrations" {
                let nonempty: bool = sqlx::query_scalar(&format!(
                    "SELECT EXISTS(SELECT 1 FROM {})",
                    identifier(table)?
                ))
                .fetch_one(&self.pool)
                .await?;
                require(!nonempty, ErrorCode::InvalidRequest)?;
            }
        }
        self.migrate()
            .await
            .map_err(|_| Error(ErrorCode::IntegrityFailure))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("LOCK TABLE server_lease IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await?;
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&mut *tx)
            .await?;
        let tables:Vec<String>=sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname=current_schema() AND tablename<>'_sqlx_migrations' ORDER BY tablename").fetch_all(&mut *tx).await?;
        require(tables == header.tables, ErrorCode::IntegrityFailure)?;
        let migrations=sqlx::query("SELECT version,encode(checksum,'hex') checksum FROM _sqlx_migrations WHERE success ORDER BY version").fetch_all(&mut *tx).await?;
        let mut hash = Sha256::new();
        for migration in migrations {
            hash.update(migration.get::<i64, _>("version").to_be_bytes());
            hash.update(migration.get::<String, _>("checksum").as_bytes());
        }
        require(
            format!("{:x}", hash.finalize()) == header.schema_digest,
            ErrorCode::IntegrityFailure,
        )?;
        for table in &tables {
            let nonempty: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS(SELECT 1 FROM {})",
                identifier(table)?
            ))
            .fetch_one(&mut *tx)
            .await?;
            require(!nonempty, ErrorCode::InvalidRequest)?;
        }
        sqlx::query("CREATE TEMP TABLE sct_recovery_rows(table_name text NOT NULL,payload jsonb NOT NULL) ON COMMIT DROP").execute(&mut *tx).await?;
        sqlx::query("CREATE TEMP TABLE sct_recovery_inventory(generation text PRIMARY KEY,root_id text NOT NULL,backend_id text NOT NULL,digest text NOT NULL,bytes bigint NOT NULL) ON COMMIT DROP").execute(&mut *tx).await?;
        let columns=sqlx::query("SELECT table_name,column_name FROM information_schema.columns WHERE table_schema=current_schema() ORDER BY table_name,ordinal_position").fetch_all(&mut *tx).await?;
        let mut column_names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for row in columns {
            column_names
                .entry(row.get("table_name"))
                .or_default()
                .insert(row.get("column_name"));
        }
        let mut catalog = BufReader::new(
            tokio::fs::File::open(package.join("catalog.jsonl"))
                .await
                .map_err(io)?,
        );
        line(&mut catalog).await?;
        let mut rows = Vec::new();
        while let Some(bytes) = line(&mut catalog).await? {
            let record: CatalogRecord = serde_json::from_slice(&bytes)?;
            require(tables.contains(&record.table), ErrorCode::IntegrityFailure)?;
            let keys: BTreeSet<String> = record
                .row
                .as_object()
                .ok_or(Error(ErrorCode::IntegrityFailure))?
                .keys()
                .cloned()
                .collect();
            require(
                column_names.get(&record.table) == Some(&keys),
                ErrorCode::IntegrityFailure,
            )?;
            rows.push(record);
            if rows.len() == 128 {
                flush_rows(&mut tx, &mut rows).await?;
            }
        }
        flush_rows(&mut tx, &mut rows).await?;
        require(
            hash_file(&package.join("catalog.jsonl")).await? == descriptor.catalog_digest.as_str(),
            ErrorCode::IntegrityFailure,
        )?;
        sqlx::query("CREATE INDEX ON sct_recovery_rows(table_name)")
            .execute(&mut *tx)
            .await?;
        let dependencies=sqlx::query("SELECT child.relname child,parent.relname parent FROM pg_constraint f JOIN pg_class child ON child.oid=f.conrelid JOIN pg_class parent ON parent.oid=f.confrelid WHERE f.contype='f' AND NOT f.condeferrable AND f.conrelid<>f.confrelid AND child.relnamespace=current_schema()::regnamespace").fetch_all(&mut *tx).await?;
        // Trusted, verified catalog import inserts immutable archive inventories.
        sqlx::query("SET LOCAL sct.archive_import = 'on'")
            .execute(&mut *tx)
            .await?;
        let mut remaining: BTreeSet<String> = tables.iter().cloned().collect();
        while !remaining.is_empty() {
            let next = remaining
                .iter()
                .find(|table| {
                    dependencies.iter().all(|d| {
                        d.get::<String, _>("child") != **table
                            || !remaining.contains(&d.get::<String, _>("parent"))
                    })
                })
                .cloned()
                .ok_or(Error(ErrorCode::IntegrityFailure))?;
            let qualified = format!("{}.{}", identifier(&schema)?, identifier(&next)?);
            let query = format!(
                "INSERT INTO {qualified} OVERRIDING SYSTEM VALUE SELECT (jsonb_populate_record(NULL::{qualified},payload)).* FROM sct_recovery_rows WHERE table_name=$1"
            );
            sqlx::query(&query).bind(&next).execute(&mut *tx).await?;
            remaining.remove(&next);
        }
        sqlx::query("SET LOCAL sct.archive_import = 'off'")
            .execute(&mut *tx)
            .await?;
        // A checkpoint may predate already burned discs. Never reuse its high-water mark.
        sqlx::query("UPDATE archive_collections SET reconciliation_required=true")
            .execute(&mut *tx)
            .await?;
        let mut inventory = BufReader::new(
            tokio::fs::File::open(package.join("inventory.jsonl"))
                .await
                .map_err(io)?,
        );
        while let Some(bytes) = line(&mut inventory).await? {
            let object: CheckpointObject = serde_json::from_slice(&bytes)?;
            sqlx::query("INSERT INTO sct_recovery_inventory VALUES($1,$2,$3,$4,$5)")
                .bind(&object.generation)
                .bind(object.root_id.as_str())
                .bind(object.backend_id.as_str())
                .bind(object.digest.as_str())
                .bind(object.bytes.get() as i64)
                .execute(&mut *tx)
                .await?;
        }
        require(
            hash_file(&package.join("inventory.jsonl")).await?
                == descriptor.inventory_digest.as_str(),
            ErrorCode::IntegrityFailure,
        )?;
        let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sct_recovery_inventory i LEFT JOIN object_copies c ON c.generation=i.generation LEFT JOIN checkpoint_objects p ON p.generation=i.generation AND p.checkpoint_id=$1 WHERE c.generation IS NULL OR p.generation IS NULL OR c.root_id<>i.root_id OR c.backend_id<>i.backend_id OR c.digest<>i.digest OR c.bytes<>i.bytes OR c.state<>'available' OR c.created_sequence>$2) OR EXISTS(SELECT 1 FROM checkpoint_objects p WHERE p.checkpoint_id=$1 AND NOT EXISTS(SELECT 1 FROM sct_recovery_inventory i WHERE i.generation=p.generation))").bind(descriptor.checkpoint_id.as_str()).bind(descriptor.generation_watermark.get() as i64).fetch_one(&mut *tx).await?;
        require(!invalid, ErrorCode::IntegrityFailure)?;
        validate_catalog(&mut tx).await?;
        // Copy one verified object at a time into fresh destinations. No production
        // destination identity or resolved credential is ever opened here.
        let backend_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM backends ORDER BY id")
            .fetch_all(&mut *tx)
            .await?;
        let mut stores = BTreeMap::new();
        for backend in backend_ids {
            let _: BackendId = backend.parse()?;
            let path = destination.join(&backend);
            private_directory(&path).await?;
            stores.insert(backend, LocalStore::new(&path, 0)?);
        }
        let mut inventory = BufReader::new(
            tokio::fs::File::open(package.join("inventory.jsonl"))
                .await
                .map_err(io)?,
        );
        while let Some(bytes) = line(&mut inventory).await? {
            let object: CheckpointObject = serde_json::from_slice(&bytes)?;
            let file = tokio::fs::File::open(checked_path(&payloads, &object).await?)
                .await
                .map_err(io)?;
            stores
                .get(object.backend_id.as_str())
                .ok_or(Error(ErrorCode::IntegrityFailure))?
                .put(&key(&object)?, object.bytes.get(), Box::pin(file))
                .await?;
        }
        for backend in stores.keys() {
            let path = destination.join(backend);
            sqlx::query("UPDATE backends SET kind='local',destination_identity=$2,inspection=jsonb_set(inspection,'{adapter_kind}','\"local\"'::jsonb) WHERE id=$1").bind(backend).bind(format!("local:{}",path.display())).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO backend_configurations(backend_id,revision,settings) VALUES($1,$2,$3) ON CONFLICT(backend_id) DO UPDATE SET revision=EXCLUDED.revision,settings=EXCLUDED.settings").bind(backend).bind(crate::error::id()).bind(serde_json::json!({"directory":path,"headroom_bytes":0})).execute(&mut *tx).await?;
        }
        reset_runtime(&mut tx).await?;
        // Imported identity values must never be reused by subsequent writes.
        let identities=sqlx::query("SELECT table_name,column_name FROM information_schema.columns WHERE table_schema=current_schema() AND (is_identity='YES' OR column_default LIKE 'nextval(%')").fetch_all(&mut *tx).await?;
        for row in identities {
            let table: String = row.get("table_name");
            let column: String = row.get("column_name");
            let qualified = format!("{}.{}", identifier(&schema)?, identifier(&table)?);
            let query = format!(
                "SELECT setval(pg_get_serial_sequence($1,$2),GREATEST(COALESCE(max({}),0),1),COALESCE(max({}),0)>0) FROM {qualified}",
                identifier(&column)?,
                identifier(&column)?
            );
            sqlx::query(&query)
                .bind(&qualified)
                .bind(&column)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "INSERT INTO management_audit(event,details) VALUES('checkpoint.recovered',$1)",
        )
        .bind(serde_json::json!({"checkpoint_id":descriptor.checkpoint_id}))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(descriptor)
    }
}

async fn validate_catalog(tx: &mut sqlx::PgConnection) -> Result<()> {
    use sqlx::Row;
    let invalid:bool=sqlx::query_scalar("WITH RECURSIVE namespace(root_id,id) AS (SELECT root_id,id FROM entries WHERE parent_id IS NULL UNION SELECT e.root_id,e.id FROM entries e JOIN namespace n ON n.root_id=e.root_id AND n.id=e.parent_id) SELECT EXISTS(SELECT 1 FROM entries e WHERE NOT EXISTS(SELECT 1 FROM namespace n WHERE n.root_id=e.root_id AND n.id=e.id)) OR EXISTS(SELECT 1 FROM entries e JOIN entries p ON p.root_id=e.root_id AND p.id=e.parent_id WHERE p.kind<>'directory') OR EXISTS(SELECT 1 FROM roots r WHERE NOT EXISTS(SELECT 1 FROM entries e WHERE e.root_id=r.id AND e.parent_id IS NULL AND e.kind='directory'))").fetch_one(&mut *tx).await?;
    require(!invalid, ErrorCode::IntegrityFailure)?;
    sqlx::query("CREATE TEMP TABLE sct_recovery_retained ON COMMIT DROP AS WITH RECURSIVE retained(root_id,digest) AS (SELECT root_id,manifest FROM file_versions WHERE NOT retired UNION SELECT e.root_id,e.child FROM content_edges e JOIN retained r ON r.root_id=e.root_id AND r.digest=e.parent) SELECT * FROM retained").execute(&mut *tx).await?;
    sqlx::query("CREATE UNIQUE INDEX ON sct_recovery_retained(root_id,digest)")
        .execute(&mut *tx)
        .await?;
    let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sct_recovery_retained r JOIN root_backends b ON b.root_id=r.root_id WHERE NOT EXISTS(SELECT 1 FROM sct_recovery_inventory i WHERE i.root_id=r.root_id AND i.digest=r.digest AND i.backend_id=b.backend_id))").fetch_one(&mut *tx).await?;
    require(!invalid, ErrorCode::IntegrityFailure)?;
    let mut root = String::new();
    let mut digest = String::new();
    loop {
        let rows=sqlx::query("SELECT o.* FROM content_objects o JOIN sct_recovery_retained r ON r.root_id=o.root_id AND r.digest=o.digest WHERE (o.root_id,o.digest)>($1,$2) ORDER BY o.root_id,o.digest LIMIT 128").bind(&root).bind(&digest).fetch_all(&mut *tx).await?;
        let done = rows.len() < 128;
        for row in rows {
            root = row.get("root_id");
            digest = row.get("digest");
            require(
                crate::integrity::shape(&row)?
                    == crate::content_catalog::children(tx, &root, &digest).await?,
                ErrorCode::IntegrityFailure,
            )?;
            let physical = if row.get::<String, _>("kind") == "chunk" {
                row.get::<i64, _>("length")
            } else {
                row.get::<Option<Vec<u8>>, _>("body")
                    .ok_or(Error(ErrorCode::IntegrityFailure))?
                    .len() as i64
            };
            let bad:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sct_recovery_inventory WHERE root_id=$1 AND digest=$2 AND bytes<>$3)").bind(&root).bind(&digest).bind(physical).fetch_one(&mut *tx).await?;
            require(!bad, ErrorCode::IntegrityFailure)?;
        }
        if done {
            break;
        }
    }
    Ok(())
}
async fn reset_runtime(tx: &mut sqlx::PgConnection) -> Result<()> {
    // Runtime ownership and client pins belong to the failed service instance.
    for table in [
        "server_lease",
        "checkpoint_barrier",
        "read_pins",
        "cursors",
        "oidc_flows",
        "reservations",
        "graph_pins",
        "graph_marks",
        "graph_scans",
    ] {
        sqlx::query(&format!("DELETE FROM {}", identifier(table)?))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE checkpoints SET state='released',revision=$1")
        .bind(crate::error::id())
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE credentials SET revoked=true")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE transfer_sessions SET state='expired',expires_at=clock_timestamp() WHERE state IN ('open','ready')").execute(&mut *tx).await?;
    sqlx::query("UPDATE tree_drafts SET state='expired',phase='terminal',expires_at=clock_timestamp() WHERE state IN ('open','ready')").execute(&mut *tx).await?;
    sqlx::query("UPDATE object_copies SET state='deleted',uncertain=false,pending_charge=false WHERE NOT EXISTS(SELECT 1 FROM sct_recovery_inventory i WHERE i.generation=object_copies.generation)").execute(&mut *tx).await?;
    sqlx::query("UPDATE allocations a SET reserved=0,occupied=(SELECT COALESCE(sum(c.bytes),0) FROM object_copies c WHERE c.allocation_id=a.id AND c.state<>'deleted'),pending_cleanup=(SELECT COALESCE(sum(c.bytes),0) FROM object_copies c WHERE c.allocation_id=a.id AND c.state<>'deleted' AND c.pending_charge)").execute(&mut *tx).await?;
    sqlx::query("UPDATE operations SET state='recovering',epoch=epoch+1,server_epoch=0,worker_expires_at=NULL,uncertain_io=false WHERE state IN ('active','recovering','cleaning_up')").execute(&mut *tx).await?;
    sqlx::query("UPDATE maintenance_jobs SET state='blocked',failure='unauthorized',worker_expires_at=NULL,next_attempt_at=NULL,revision=$1 WHERE state IN ('queued','running','retry_wait')").bind(crate::error::id()).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO configuration_state(singleton) VALUES(true) ON CONFLICT DO NOTHING")
        .execute(&mut *tx)
        .await?;
    Ok(())
}
