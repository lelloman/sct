//! Full original-disc read-back observations for catalog import.
use crate::*;
use sqlx::Row;
impl ArchiveIndex {
    /// Unlike verify_media for a later recovery target, this scans the original
    /// disc's entire frozen payload inventory. It never certifies only a subset.
    pub async fn verify_disc_report(
        &mut self,
        media: &Path,
        details: &mut impl Write,
    ) -> Result<ArchiveMediaVerification> {
        let dir = SafeDir::open(media)?;
        let desc = descriptor(&dir)?;
        ensure(
            desc.collection_id == self.descriptor.collection_id
                && desc.set_id == self.descriptor.set_id
                && desc.set_sequence == self.descriptor.set_sequence
                && desc.index_sha256 == self.descriptor.index_sha256
                && desc.bundle_sha256 == self.descriptor.bundle_sha256
                && desc.first_disc == self.descriptor.first_disc
                && desc.last_disc == self.descriptor.last_disc
                && desc.index_bytes == self.descriptor.index_bytes
                && desc.index_records == self.descriptor.index_records,
            "read-back requires this disc's original snapshot index, not a later recovery target",
        )?;
        let identity = sqlx::query("SELECT number,set_sequence FROM discs WHERE id=?")
            .bind(&desc.disc_id)
            .fetch_optional(&mut self.connection)
            .await?
            .ok_or("disc not in original index")?;
        ensure(
            identity.get::<i64, _>(0) as u64 == desc.number
                && identity.get::<i64, _>(1) as u64 == desc.set_sequence,
            "disc identity mismatch",
        )?;
        let metadata_verified = dir
            .file("snapshot.jsonl")
            .and_then(|file| hash_reader(file, desc.index_bytes))
            .is_ok_and(|hash| hash == desc.index_sha256.as_str())
            && verify_bundle(&dir, &desc).is_ok();
        let mut inventory = Sha256::new();
        let mut expected = 0u64;
        let mut verified = 0u64;
        let mut bytes = 0u64;
        let mut after = (String::new(), String::new());
        loop {
            let rows=sqlx::query("SELECT root,digest,length FROM placements WHERE disc=? AND (root,digest)>(?,?) ORDER BY root,digest LIMIT 128").bind(&desc.disc_id).bind(&after.0).bind(&after.1).fetch_all(&mut self.connection).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let object = ArchiveObject {
                    root_id: row.get::<String, _>(0).parse()?,
                    digest: row.get::<String, _>(1).parse()?,
                    length: (row.get::<i64, _>(2) as u64).try_into()?,
                };
                after = (
                    object.root_id.as_str().into(),
                    object.digest.as_str().into(),
                );
                inventory.update(canonical(&object)?);
                inventory.update(b"\n");
                expected += 1;
                let good = dir
                    .file(&object.relative_path())
                    .and_then(|file| hash_reader(file, object.length.get()))
                    .is_ok_and(|hash| hash == object.digest.as_str());
                if good {
                    verified += 1;
                    bytes += object.length.get();
                }
                serde_json::to_writer(
                    &mut *details,
                    &serde_json::json!({"disc_id":desc.disc_id,"object":object,"verified":good}),
                )?;
                details.write_all(b"\n")?;
            }
        }
        Ok(ArchiveMediaVerification {
            collection_id: desc.collection_id.parse()?,
            disc_id: desc.disc_id.parse()?,
            index_sha256: desc.index_sha256,
            bundle_sha256: desc.bundle_sha256,
            payload_inventory_sha256: format!("{:x}", inventory.finalize()).parse()?,
            expected_objects: expected.try_into()?,
            verified_objects: verified.try_into()?,
            verified_bytes: bytes.try_into()?,
            metadata_verified,
            verifier_version: format!("sct-recover-{}", env!("CARGO_PKG_VERSION")).parse()?,
        })
    }
}
