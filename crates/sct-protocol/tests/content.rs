use sct_protocol::*;
use std::io::Cursor;

#[test]
fn canonical_empty_file_and_rejection() {
    let file = FileManifest {
        format: 1,
        profile: CHUNK_PROFILE.into(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
        height: 0,
        tree: None,
    };
    file.validate().unwrap();
    let bytes = canonical(&file).unwrap();
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        "{\"chunks\":0,\"format\":1,\"height\":0,\"length\":0,\"profile\":\"fastcdc-v2020-3.2.1-n1-s0-256k-1m-4m\",\"tree\":null}"
    );
    assert_eq!(parse_canonical::<FileManifest>(&bytes).unwrap(), file);
    for invalid in [
        format!(" {}", String::from_utf8(bytes.clone()).unwrap()),
        String::from_utf8(bytes)
            .unwrap()
            .replace("\"length\":0", "\"length\":9007199254740992"),
        "{\"format\":1,\"format\":1}".into(),
    ] {
        assert!(parse_canonical::<FileManifest>(invalid.as_bytes()).is_err());
    }
    assert!(parse_canonical::<FileManifest>(&vec![b' '; MANIFEST_BYTES + 1]).is_err());
    assert_eq!(chunker(Cursor::new([])).count(), 0);
}

#[test]
fn repeated_chunks_and_bounds() {
    let c = ChunkReference {
        digest: digest(b"x").parse().unwrap(),
        length: 1.try_into().unwrap(),
    };
    let page = ManifestPage::Chunks {
        format: 1,
        chunks: vec![c.clone(), c.clone()],
    };
    page.validate().unwrap();
    assert_eq!(
        parse_canonical::<ManifestPage>(&canonical(&page).unwrap()).unwrap(),
        page
    );
    assert!(
        ManifestPage::Chunks {
            format: 1,
            chunks: vec![c; MANIFEST_FANOUT + 1]
        }
        .validate()
        .is_err()
    );
    assert!(
        ManifestPage::Chunks {
            format: 2,
            chunks: vec![]
        }
        .validate()
        .is_err()
    );
}

#[test]
fn pinned_fastcdc_fixture() {
    // xorshift64, fixed unsigned wrap semantics, low byte per iteration.
    let mut state = 0x123456789abcdef0u64;
    let bytes: Vec<u8> = (0..12 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    let chunks: Vec<_> = chunker(Cursor::new(&bytes))
        .map(|v| {
            let c = v.unwrap();
            (c.length, digest(&c.data))
        })
        .collect();
    let actual = serde_json::to_string(&chunks).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::from_str::<serde_json::Value>(include_str!(
            "../../../contracts/fixtures/chunk-profile.json"
        ))
        .unwrap()
    );
}
