use sct_archive::*;
use sct_protocol::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sct-restore-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn ten_discs(s: &Scratch) -> Vec<PathBuf> {
    let root: RootId = "root".parse().unwrap();
    let version: VersionId = "version".parse().unwrap();
    let objects: Vec<_> = (1..=10)
        .map(|n| ArchiveObject {
            root_id: root.clone(),
            digest: digest(&[n; 17]).parse().unwrap(),
            length: 17.try_into().unwrap(),
        })
        .collect();
    let mut records = vec![ArchiveRecord::Snapshot {
        format: 1,
        collection_id: "collection".into(),
        set_id: "set".into(),
        set_sequence: 1,
        first_disc: 1,
        last_disc: 10,
        sources: vec![ArchiveSource {
            root_id: root.clone(),
            version_id: version.clone(),
            path: "/".parse().unwrap(),
        }],
    }];
    for n in 1..=10 {
        records.push(ArchiveRecord::Disc {
            disc_id: format!("disc-{n}"),
            number: n,
            set_sequence: 1,
        });
    }
    records.push(ArchiveRecord::Directory {
        path: "/".parse().unwrap(),
    });
    records.push(ArchiveRecord::Directory {
        path: "/empty".parse().unwrap(),
    });
    let mut files: Vec<(String, Vec<usize>)> = (0..10)
        .map(|n| (format!("/file-{n:02}"), vec![n]))
        .collect();
    files.push(("/shared".into(), vec![9]));
    files.push(("/spanning".into(), vec![0, 4, 9]));
    files.push(("/zero".into(), vec![]));
    for (path, chunks) in &files {
        records.push(ArchiveRecord::File {
            path: path.parse().unwrap(),
            root_id: root.clone(),
            version_id: version.clone(),
            length: ((chunks.len() * 17) as u64).try_into().unwrap(),
        });
    }
    for (path, chunks) in &files {
        for (i, n) in chunks.iter().enumerate() {
            records.push(ArchiveRecord::Extent {
                path: path.parse().unwrap(),
                offset: ((i * 17) as u64).try_into().unwrap(),
                object: objects[*n].clone(),
            });
        }
    }
    let mut placements: Vec<_> = objects
        .iter()
        .enumerate()
        .map(|(n, object)| ArchiveRecord::Placement {
            object: object.clone(),
            disc_id: format!("disc-{}", n + 1),
        })
        .collect();
    placements.sort_by_key(|r| match r {
        ArchiveRecord::Placement { object, .. } => object.digest.as_str().to_owned(),
        _ => unreachable!(),
    });
    records.extend(placements);
    let mut index = Vec::new();
    for r in &records {
        index.extend(canonical(r).unwrap());
        index.push(b'\n');
    }
    let instruction = b"Format fixture; not an executable recovery bundle.\n";
    let bundle = canonical(&ArchiveBundle {
        format: 1,
        version: "fixture".into(),
        platform: "fixture".into(),
        files: vec![ArchiveBundleFile {
            path: "START-HERE.txt".into(),
            length: (instruction.len() as u64).try_into().unwrap(),
            sha256: digest(instruction).parse().unwrap(),
        }],
    })
    .unwrap();
    (1..=10)
        .map(|n| {
            let dir = s.dir(&format!("disc-{n}"));
            fs::create_dir(dir.join("recovery")).unwrap();
            fs::write(dir.join("START-HERE.txt"), instruction).unwrap();
            fs::write(dir.join("recovery/bundle.json"), &bundle).unwrap();
            fs::write(dir.join("snapshot.jsonl"), &index).unwrap();
            let desc = ArchiveDiscDescriptor {
                format: 1,
                collection_id: "collection".into(),
                set_id: "set".into(),
                set_sequence: 1,
                disc_id: format!("disc-{n}"),
                number: n,
                first_disc: 1,
                last_disc: 10,
                index_sha256: digest(&index).parse().unwrap(),
                index_bytes: index.len() as u64,
                index_records: records.len() as u64,
                bundle_sha256: digest(&bundle).parse().unwrap(),
            };
            fs::write(dir.join("disc.json"), canonical(&desc).unwrap()).unwrap();
            let object = dir.join(objects[n as usize - 1].relative_path());
            fs::create_dir_all(object.parent().unwrap()).unwrap();
            fs::write(object, [n as u8; 17]).unwrap();
            dir
        })
        .collect()
}
fn cli(args: &[&Path]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sct-recover"))
        .args(args)
        .output()
        .unwrap()
}
#[tokio::test]
async fn missing_last_disc_bootstrap_nine_partial_resume_and_conflicts() {
    let s = Scratch::new();
    let discs = ten_discs(&s);
    let cache = s.dir("cache");
    let out = s.dir("out");
    let db = s.0.join("work.sqlite");
    let mut index = ArchiveIndex::create(&discs[8], &db).await.unwrap();
    assert!(
        ArchiveIndex::open(&db).await.is_err(),
        "concurrent recovery must be fenced"
    );
    for n in (0..9).rev() {
        index
            .ingest(&discs[n], &cache, &mut Vec::new())
            .await
            .unwrap();
    }
    index
        .ingest(&discs[8], &cache, &mut Vec::new())
        .await
        .unwrap();
    let mut report = Vec::new();
    let summary = index
        .restore_to(&cache, &out, false, &mut report)
        .await
        .unwrap();
    assert!(!summary.complete);
    assert_eq!(summary.unavailable_files, 3);
    assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    let summary = index
        .restore_to(&cache, &out, true, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(summary.restored_files, 10);
    assert_eq!(summary.unavailable_files, 3);
    assert!(!summary.complete);
    assert!(out.join("empty").is_dir());
    assert_eq!(fs::read(out.join("zero")).unwrap(), b"");
    for n in 0..9 {
        assert_eq!(
            fs::read(out.join(format!("file-{n:02}"))).unwrap(),
            [n as u8 + 1; 17]
        );
    }
    assert!(!out.join("spanning").exists());
    assert!(!out.join("shared").exists());
    let report = String::from_utf8(report).unwrap();
    assert!(report.contains("/spanning"));
    assert!(report.contains("/shared"));
    assert!(report.contains("/file-09"));
    drop(index);
    // Each CLI invocation is a new process and reopens persistent work/cache state.
    let result = cli(&[Path::new("ingest"), &discs[9], &db, &cache]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = cli(&[Path::new("restore"), &db, &cache, &out]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read(out.join("spanning")).unwrap(),
        [[1; 17], [5; 17], [10; 17]].concat()
    );
    fs::write(out.join("file-00"), b"unrelated data").unwrap();
    let result = cli(&[Path::new("restore"), &db, &cache, &out]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(out.join("file-00")).unwrap(), b"unrelated data");
}
#[tokio::test]
async fn middle_disc_corrupt_cache_and_destination_symlinks() {
    let s = Scratch::new();
    let discs = ten_discs(&s);
    let cache = s.dir("cache");
    let out = s.dir("out");
    let mut index = ArchiveIndex::create(&discs[0], &s.0.join("work"))
        .await
        .unwrap();
    for (n, disc) in discs.iter().enumerate() {
        if n != 4 {
            index.ingest(disc, &cache, &mut Vec::new()).await.unwrap();
        }
    }
    let summary = index
        .restore_to(&cache, &out, true, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(summary.unavailable_files, 2);
    assert!(!out.join("spanning").exists());
    let victim = ArchiveObject {
        root_id: "root".parse().unwrap(),
        digest: digest(&[1; 17]).parse().unwrap(),
        length: 17.try_into().unwrap(),
    };
    fs::write(cache.join(victim.relative_path()), [2; 17]).unwrap();
    let summary = index
        .restore_to(&cache, &out, true, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(summary.unavailable_files, 3);
    let outside = s.dir("outside");
    let unsafe_out = s.dir("unsafe");
    std::os::unix::fs::symlink(&outside, unsafe_out.join("empty")).unwrap();
    assert!(
        index
            .restore_to(&cache, &unsafe_out, true, &mut Vec::new())
            .await
            .is_err()
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    let link = s.0.join("link");
    std::os::unix::fs::symlink(&out, &link).unwrap();
    assert!(
        index
            .restore_to(&cache, &link, true, &mut Vec::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn killed_restore_resumes_without_exposing_partial_files() {
    use std::{
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };
    let s = Scratch::new();
    let discs = ten_discs(&s);
    // A single 4 MiB object repeated 64 times keeps the media small while making
    // a sufficiently long restore to kill during a destination write.
    let payload = vec![42u8; 4 * 1024 * 1024];
    let object = ArchiveObject {
        root_id: "root".parse().unwrap(),
        digest: digest(&payload).parse().unwrap(),
        length: (payload.len() as u64).try_into().unwrap(),
    };
    let mut records: Vec<ArchiveRecord> = fs::read_to_string(discs[0].join("snapshot.jsonl"))
        .unwrap()
        .lines()
        .map(|l| archive_record(l.as_bytes()).unwrap())
        .collect();
    records.retain(|r| {
        matches!(
            r,
            ArchiveRecord::Snapshot { .. }
                | ArchiveRecord::Disc { .. }
                | ArchiveRecord::Directory { .. }
        )
    });
    records.push(ArchiveRecord::File {
        path: "/large".parse().unwrap(),
        root_id: "root".parse().unwrap(),
        version_id: "version".parse().unwrap(),
        length: (64 * object.length.get()).try_into().unwrap(),
    });
    for n in 0..64 {
        records.push(ArchiveRecord::Extent {
            path: "/large".parse().unwrap(),
            offset: (n * object.length.get()).try_into().unwrap(),
            object: object.clone(),
        });
    }
    records.push(ArchiveRecord::Placement {
        object: object.clone(),
        disc_id: "disc-1".into(),
    });
    let mut bytes = Vec::new();
    for r in &records {
        bytes.extend(canonical(r).unwrap());
        bytes.push(b'\n');
    }
    let mut desc: ArchiveDiscDescriptor =
        parse_canonical(&fs::read(discs[0].join("disc.json")).unwrap()).unwrap();
    desc.index_sha256 = digest(&bytes).parse().unwrap();
    desc.index_bytes = bytes.len() as u64;
    desc.index_records = records.len() as u64;
    fs::write(discs[0].join("disc.json"), canonical(&desc).unwrap()).unwrap();
    fs::write(discs[0].join("snapshot.jsonl"), bytes).unwrap();
    fs::write(discs[0].join(object.relative_path()), &payload).unwrap();
    let cache = s.dir("cache");
    let out = s.dir("out");
    let db = s.0.join("work");
    let mut index = ArchiveIndex::create(&discs[0], &db).await.unwrap();
    index
        .ingest(&discs[0], &cache, &mut Vec::new())
        .await
        .unwrap();
    drop(index);
    let mut child = Command::new(env!("CARGO_BIN_EXE_sct-recover"))
        .arg("restore")
        .arg(&db)
        .arg(&cache)
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if fs::read_dir(&out).unwrap().filter_map(|e| e.ok()).any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".sct-recovery-")
                && e.metadata().unwrap().len() > 0
        }) {
            break;
        }
        assert!(Instant::now() < deadline);
        assert!(child.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(1));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!out.join("large").exists());
    let result = cli(&[Path::new("restore"), &db, &cache, &out]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut file = fs::File::open(out.join("large")).unwrap();
    use std::io::Read;
    for _ in 0..64 {
        let mut actual = vec![0; payload.len()];
        file.read_exact(&mut actual).unwrap();
        assert_eq!(digest(&actual), object.digest.as_str());
    }
    assert_eq!(file.read(&mut [0]).unwrap(), 0);
}
