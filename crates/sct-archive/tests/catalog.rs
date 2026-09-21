use sct_archive::{ArchiveCatalog, ArchiveIndex};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sct-catalog-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture(number: u32) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../contracts/fixtures/disc-archive/disc-{number}"
    ))
}
async fn export(catalog: &mut ArchiveCatalog, full: bool) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    catalog.export(&mut out, full).await.unwrap();
    out.split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}
#[tokio::test]
async fn scans_preserve_separate_recovery_points_without_inventing_missing_history() {
    let scratch = Scratch::new();
    let mut catalog = ArchiveCatalog::open(&scratch.0.join("catalog.sqlite"))
        .await
        .unwrap();
    let index = ArchiveIndex::create(&fixture(5), &scratch.0.join("5.sqlite"))
        .await
        .unwrap();
    catalog.scan(&fixture(5), &index).await.unwrap();
    let partial = export(&mut catalog, true).await;
    assert_eq!(partial[0]["coverage"], "observed_metadata_only");
    assert_eq!(partial[0]["unknown_later_sets"], true);
    assert_eq!(partial[0]["physical_copy_observations_available"], false);
    assert_eq!(
        partial
            .iter()
            .filter(|r| r["kind"] == "recovery_point")
            .count(),
        1
    );
    let old = partial
        .iter()
        .find(|r| r["kind"] == "logical_disc" && r["number"] == 1)
        .unwrap();
    assert_eq!(old["metadata_scanned"], false);
    assert_eq!(old["original_snapshot_index_known"], false);
    assert_eq!(old["physical_copy_health"], "unknown");
    catalog.scan(&fixture(5), &index).await.unwrap();
    assert_eq!(partial, export(&mut catalog, true).await);
    assert!(catalog.scan(&fixture(1), &index).await.is_err());
    assert_eq!(partial, export(&mut catalog, true).await);
    for number in [1, 6] {
        let index = ArchiveIndex::create(
            &fixture(number),
            &scratch.0.join(format!("{number}.sqlite")),
        )
        .await
        .unwrap();
        catalog.scan(&fixture(number), &index).await.unwrap();
    }
    let full = export(&mut catalog, true).await;
    assert_eq!(
        full.iter()
            .filter(|r| r["kind"] == "recovery_point")
            .count(),
        3
    );
    assert_eq!(
        full.iter().filter(|r| r["kind"] == "logical_disc").count(),
        6
    );
    assert_eq!(
        full.iter()
            .filter(|r| r["kind"] == "logical_disc" && r["metadata_scanned"] == true)
            .count(),
        3
    );
    for number in [1, 5, 6] {
        let desc: sct_protocol::ArchiveDiscDescriptor =
            serde_json::from_slice(&fs::read(fixture(number).join("disc.json")).unwrap()).unwrap();
        let mut records = Vec::new();
        for record in full
            .iter()
            .filter(|r| r["kind"] == "index_record" && r["set_id"] == desc.set_id)
        {
            records.extend(sct_protocol::canonical(&record["record"]).unwrap());
            records.push(b'\n');
        }
        assert_eq!(
            records,
            fs::read(fixture(number).join("snapshot.jsonl")).unwrap()
        );
    }
    drop(catalog);
    let mut reopened = ArchiveCatalog::open(&scratch.0.join("catalog.sqlite"))
        .await
        .unwrap();
    assert_eq!(full, export(&mut reopened, true).await);
}

#[test]
fn cli_scan_export_and_missing_catalog_are_explicit() {
    use std::process::Command;
    let scratch = Scratch::new();
    let binary = env!("CARGO_BIN_EXE_sct-recover");
    let catalog = scratch.0.join("catalog.sqlite");
    let absent = Command::new(binary)
        .arg("catalog-inspect")
        .arg(&catalog)
        .output()
        .unwrap();
    assert!(!absent.status.success());
    assert!(!catalog.exists());
    for number in [3, 4, 6] {
        let result = Command::new(binary)
            .arg("catalog-scan")
            .arg(fixture(number))
            .arg(&catalog)
            .arg(scratch.0.join(format!("{number}.sqlite")))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let inspected = Command::new(binary)
        .arg("catalog-inspect")
        .arg(&catalog)
        .output()
        .unwrap();
    assert!(inspected.status.success());
    let rows: Vec<serde_json::Value> = String::from_utf8(inspected.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        rows.iter()
            .filter(|r| r["kind"] == "recovery_point")
            .count(),
        3
    );
    assert!(!rows.iter().any(|r| r["kind"] == "index_record"));
    let exported = Command::new(binary)
        .arg("catalog-export")
        .arg(&catalog)
        .output()
        .unwrap();
    assert!(exported.status.success());
    assert!(
        String::from_utf8(exported.stdout)
            .unwrap()
            .contains("\"kind\":\"index_record\"")
    );
}
