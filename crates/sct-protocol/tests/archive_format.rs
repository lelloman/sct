use sct_protocol::*;
use std::{collections::BTreeMap, fs, path::Path};

#[test]
fn canonical_archive_fixture_history_and_every_disc_index() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures/disc-archive");
    for line in fs::read_to_string(base.join("SHA256SUMS")).unwrap().lines() {
        let (hash, path) = line.split_once("  ").unwrap();
        assert_eq!(digest(&fs::read(base.join(path)).unwrap()), hash);
    }
    let mut indexes = BTreeMap::new();
    for number in 1..=6 {
        let dir = base.join(format!("disc-{number}"));
        let descriptor: ArchiveDiscDescriptor =
            parse_canonical(&fs::read(dir.join("disc.json")).unwrap()).unwrap();
        descriptor.validate().unwrap();
        let index = fs::read(dir.join("snapshot.jsonl")).unwrap();
        assert_eq!(descriptor.index_sha256.as_str(), digest(&index));
        assert_eq!(descriptor.index_bytes, index.len() as u64);
        let records: Vec<_> = index
            .split(|&c| c == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| archive_record(line).unwrap())
            .collect();
        assert_eq!(descriptor.index_records, records.len() as u64);
        if let Some(previous) = indexes.insert(descriptor.set_id.clone(), index.clone()) {
            assert_eq!(previous, index);
        }
        let bundle_bytes = fs::read(dir.join("recovery/bundle.json")).unwrap();
        assert_eq!(descriptor.bundle_sha256.as_str(), digest(&bundle_bytes));
        let bundle: ArchiveBundle = parse_canonical(&bundle_bytes).unwrap();
        bundle.validate().unwrap();
        let mut placements = BTreeMap::new();
        let mut expected = BTreeMap::new();
        let mut reconstructed = BTreeMap::<String, Vec<u8>>::new();
        for record in &records {
            if let ArchiveRecord::Placement { object, disc_id } = record {
                placements.insert(object.digest.as_str(), disc_id);
            }
            if let ArchiveRecord::File { path, length, .. } = record {
                expected.insert(path.as_str(), length.get());
            }
        }
        for record in records.iter() {
            if let ArchiveRecord::Extent {
                path,
                offset,
                object,
            } = record
            {
                let disc = placements[object.digest.as_str()];
                let bytes = fs::read(base.join(disc).join(object.relative_path())).unwrap();
                assert_eq!(digest(&bytes), object.digest.as_str());
                let file = reconstructed.entry(path.as_str().into()).or_default();
                assert_eq!(offset.get(), file.len() as u64);
                file.extend(bytes);
            }
        }
        for (path, length) in expected {
            assert_eq!(reconstructed[path].len() as u64, length);
        }
        let paths: Vec<_> = reconstructed.keys().map(String::as_str).collect();
        if descriptor.set_sequence == 1 {
            assert!(paths.contains(&"/gone"));
        }
        if descriptor.set_sequence == 2 {
            assert!(!paths.contains(&"/gone"));
            assert!(paths.contains(&"/renamed"));
        }
        if descriptor.set_sequence == 3 {
            assert!(!placements.values().any(|d| d.as_str() == "disc-1"));
        }
    }
}

#[test]
fn archive_wire_rejects_unsafe_paths_versions_and_bounds() {
    for path in [
        "/../escape",
        "/a/../../escape",
        "relative",
        "/a//b",
        "/a/\0",
    ] {
        assert!(path.parse::<RootPath>().is_err());
    }
    assert!(archive_record(&vec![b' '; ARCHIVE_RECORD_BYTES + 1]).is_err());
    assert!(archive_record(br#"{"kind":"directory","path":"/","unexpected":true}"#).is_err());
    let record = ArchiveRecord::Snapshot {
        format: 2,
        collection_id: "a".into(),
        set_id: "b".into(),
        set_sequence: 1,
        first_disc: 1,
        last_disc: 1,
        sources: vec![ArchiveSource {
            root_id: "r".parse().unwrap(),
            version_id: "v".parse().unwrap(),
            path: "/".parse().unwrap(),
        }],
    };
    assert!(record.validate().is_err());
    assert!(archive_id("../disc").is_err());
    assert!(archive_path(&format!("/{}", vec!["a"; 129].join("/")).parse().unwrap()).is_err());
    let object = ArchiveObject {
        root_id: "r".parse().unwrap(),
        digest: digest(b"x").parse().unwrap(),
        length: 0.try_into().unwrap(),
    };
    assert!(object.validate().is_err());
}
