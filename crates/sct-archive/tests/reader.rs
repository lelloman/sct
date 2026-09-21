use sct_archive::*;
use sct_protocol::*;
use std::{
    fs,
    path::{Path, PathBuf},
};
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("sct-archive-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture(n: u32) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../contracts/fixtures/disc-archive/disc-{n}"))
}
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let e = entry.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}
fn change_index(disc: &Path, change: impl FnOnce(&mut Vec<ArchiveRecord>)) {
    let bytes = fs::read(disc.join("snapshot.jsonl")).unwrap();
    let mut records: Vec<_> = bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| archive_record(l).unwrap())
        .collect();
    change(&mut records);
    let mut bytes = Vec::new();
    for record in &records {
        bytes.extend(canonical(record).unwrap());
        bytes.push(b'\n');
    }
    fs::write(disc.join("snapshot.jsonl"), &bytes).unwrap();
    let mut d: ArchiveDiscDescriptor =
        parse_canonical(&fs::read(disc.join("disc.json")).unwrap()).unwrap();
    d.index_sha256 = digest(&bytes).parse().unwrap();
    d.index_bytes = bytes.len() as u64;
    d.index_records = records.len() as u64;
    fs::write(disc.join("disc.json"), canonical(&d).unwrap()).unwrap();
}
#[tokio::test]
async fn every_disc_bootstraps_the_same_set_and_verifies_reused_media() {
    let scratch = Scratch::new();
    let mut set_inspections = std::collections::BTreeMap::new();
    for number in 1..=6 {
        let mut index = ArchiveIndex::create(
            &fixture(number),
            &scratch.0.join(format!("{number}.sqlite")),
        )
        .await
        .unwrap();
        let mut inspection = Vec::new();
        index.inspect(&mut inspection).await.unwrap();
        let contents = inspection
            .splitn(2, |&b| b == b'\n')
            .nth(1)
            .unwrap()
            .to_vec();
        if let Some(old) = set_inspections.insert(index.descriptor.set_id.clone(), contents.clone())
        {
            assert_eq!(old, contents);
        }
        let needed: Vec<String> = sqlx::query_scalar("SELECT id FROM discs ORDER BY number DESC")
            .fetch_all(&mut index.connection)
            .await
            .unwrap();
        for id in needed {
            let n = id.strip_prefix("disc-").unwrap().parse().unwrap();
            let mut report = Vec::new();
            let result = index.verify_media(&fixture(n), &mut report).await.unwrap();
            assert!(result.complete(), "{report:?}");
            assert_eq!(result.unavailable_objects, 0);
            // A second physical copy of the same logical medium is valid.
            assert!(
                index
                    .verify_media(&fixture(n), &mut std::io::sink())
                    .await
                    .unwrap()
                    .complete()
            );
        }
    }
}
#[tokio::test]
async fn invalid_graphs_fail_before_a_usable_index_is_committed() {
    for case in 0..8 {
        let scratch = Scratch::new();
        let disc = scratch.0.join("disc");
        copy(&fixture(1), &disc);
        change_index(&disc, |rows| match case {
            0 => {
                rows.retain(|r| !matches!(r, ArchiveRecord::Placement { .. }));
            }
            1 => {
                rows.retain(|r| !matches!(r,ArchiveRecord::Directory{path} if path.as_str()=="/"));
            }
            2 => {
                for r in rows {
                    if let ArchiveRecord::Extent { offset, .. } = r {
                        *offset = 1.try_into().unwrap();
                        break;
                    }
                }
            }
            3 => {
                for r in rows {
                    if let ArchiveRecord::Extent { object, .. } = r {
                        object.root_id = "different-root".parse().unwrap();
                        break;
                    }
                }
            }
            4 => {
                rows.push(rows.last().unwrap().clone());
            }
            5 => {
                for r in rows {
                    if let ArchiveRecord::Disc { set_sequence, .. } = r {
                        *set_sequence = 999;
                        break;
                    }
                }
            }
            6 => {
                for r in rows {
                    if let ArchiveRecord::Placement { object, .. } = r {
                        object.length = 42.try_into().unwrap();
                        break;
                    }
                }
            }
            _ => {
                for r in rows {
                    if let ArchiveRecord::File { length, .. } = r {
                        *length = 99.try_into().unwrap();
                        break;
                    }
                }
            }
        });
        let db = scratch.0.join("index.sqlite");
        assert!(
            ArchiveIndex::create(&disc, &db).await.is_err(),
            "case {case}"
        );
        assert!(ArchiveIndex::open(&db).await.is_err());
    }
}
#[tokio::test]
async fn corrupt_missing_symlink_and_wrong_discs_cannot_verify() {
    let scratch = Scratch::new();
    let disc = scratch.0.join("disc");
    copy(&fixture(1), &disc);
    let mut index = ArchiveIndex::create(&fixture(1), &scratch.0.join("index.sqlite"))
        .await
        .unwrap();
    let object = ArchiveObject {
        root_id: "root-a".parse().unwrap(),
        digest: digest(b"A").parse().unwrap(),
        length: 1.try_into().unwrap(),
    };
    let file = disc.join(object.relative_path());
    for case in 0..3 {
        if case == 0 {
            fs::write(&file, b"Z").unwrap();
        }
        if case == 1 {
            fs::remove_file(&file).unwrap();
        }
        if case == 2 {
            std::os::unix::fs::symlink("/etc/passwd", &file).unwrap();
        }
        let result = index
            .verify_media(&disc, &mut std::io::sink())
            .await
            .unwrap();
        assert!(!result.complete());
        assert_eq!(result.unavailable_objects, 1);
    }
    let mut d: ArchiveDiscDescriptor =
        parse_canonical(&fs::read(disc.join("disc.json")).unwrap()).unwrap();
    d.collection_id = "other".into();
    fs::write(disc.join("disc.json"), canonical(&d).unwrap()).unwrap();
    assert!(
        index
            .verify_media(&disc, &mut std::io::sink())
            .await
            .is_err()
    );
    fs::write(disc.join("snapshot.jsonl"), b"{}\n").unwrap();
    assert!(
        ArchiveIndex::create(&disc, &scratch.0.join("broken.sqlite"))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn bounded_lines_and_noncanonical_bytes_are_rejected() {
    for bytes in [
        vec![b'x'; ARCHIVE_RECORD_BYTES + 2],
        b"{\"kind\":\"directory\",\"path\":\"/\"}\r\n".to_vec(),
        b"{}".to_vec(),
    ] {
        let scratch = Scratch::new();
        let disc = scratch.0.join("disc");
        copy(&fixture(1), &disc);
        fs::write(disc.join("snapshot.jsonl"), bytes).unwrap();
        assert!(
            ArchiveIndex::create(&disc, &scratch.0.join("index.sqlite"))
                .await
                .is_err()
        );
    }
}

#[test]
fn cli_indexes_large_catalog_with_bounded_resident_memory() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let scratch = Scratch::new();
    let disc = scratch.0.join("disc");
    copy(&fixture(1), &disc);
    change_index(&disc, |rows| {
        let mut directories: Vec<_> = (0..50_000)
            .map(|n| ArchiveRecord::Directory {
                path: format!("/large-{n:06}").parse().unwrap(),
            })
            .collect();
        let begin = rows
            .iter()
            .position(|r| {
                matches!(
                    r,
                    ArchiveRecord::Directory { .. } | ArchiveRecord::File { .. }
                )
            })
            .unwrap();
        let end = rows
            .iter()
            .position(|r| matches!(r, ArchiveRecord::Extent { .. }))
            .unwrap();
        directories.extend(rows.drain(begin..end));
        directories.sort_by_key(|r| match r {
            ArchiveRecord::Directory { path } | ArchiveRecord::File { path, .. } => {
                path.as_str().to_owned()
            }
            _ => unreachable!(),
        });
        rows.splice(begin..begin, directories);
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_sct-recover"))
        .arg("inspect")
        .arg(&disc)
        .arg(scratch.0.join("large.sqlite"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let mut peak = 0u64;
    loop {
        if let Ok(status) = fs::read_to_string(format!("/proc/{}/status", child.id()))
            && let Some(kib) = status
                .lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse::<u64>().ok())
        {
            peak = peak.max(kib);
        }
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "reader failed: {:?}",
                child.wait_with_output().unwrap()
            );
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("indexing exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        peak > 0 && peak < 128 * 1024,
        "unbounded reader RSS: {peak} KiB"
    );
    println!(
        "archive_reader entries=50005 sampled_peak_rss_kib={peak} elapsed_ms={}",
        start.elapsed().as_millis()
    );
}
