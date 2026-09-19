use sct_protocol::*;
use serde_json::json;
fn entry(id: &str, name: &str) -> DirectoryEntry {
    DirectoryEntry {
        id: id.parse().unwrap(),
        name: name.parse().unwrap(),
        kind: EntryKind::Directory,
        manifest: digest(b"empty").parse().unwrap(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
    }
}
#[test]
fn empty_directory_is_canonical_and_not_an_empty_file() {
    let tree = DirectoryManifest {
        format: 1,
        kind: DirectoryObjectKind::Directory,
        entries: 0.try_into().unwrap(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
        height: 0,
        tree: None,
    };
    tree.validate().unwrap();
    let bytes = canonical(&tree).unwrap();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        r#"{"chunks":0,"entries":0,"format":1,"height":0,"kind":"directory","length":0,"tree":null}"#
    );
    assert_eq!(parse_canonical::<DirectoryManifest>(&bytes).unwrap(), tree);
    assert!(parse_canonical::<FileManifest>(&bytes).is_err());
    let mut bad = tree;
    bad.entries = 1.try_into().unwrap();
    assert!(bad.validate().is_err());
}
#[test]
fn tree_pages_preserve_identity_and_reject_ambiguity() {
    let mut entries = vec![entry("a", "a"), entry("b", "b")];
    let page = DirectoryPage::DirectoryEntries {
        format: 1,
        entries: entries.clone(),
    };
    page.validate().unwrap();
    let hash = digest(&canonical(&page).unwrap());
    entries[0].name = "renamed".parse().unwrap();
    entries.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    assert_eq!(entries[1].id.as_str(), "a");
    assert_ne!(
        hash,
        digest(&canonical(&DirectoryPage::DirectoryEntries { format: 1, entries }).unwrap())
    );
    for entries in [
        vec![entry("a", "b"), entry("b", "a")],
        vec![entry("a", "a"), entry("b", "a")],
        vec![entry("a", "a"), entry("a", "b")],
        vec![],
        vec![entry("a", "a"); 1025],
    ] {
        assert!(
            DirectoryPage::DirectoryEntries { format: 1, entries }
                .validate()
                .is_err()
        );
    }
    let p = DirectoryPageReference {
        digest: digest(b"leaf").parse().unwrap(),
        first: "a".parse().unwrap(),
        last: "z".parse().unwrap(),
        entries: 1.try_into().unwrap(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
    };
    assert!(
        DirectoryPage::DirectoryBranch {
            format: 1,
            height: 1,
            pages: vec![p.clone(), p.clone()]
        }
        .validate()
        .is_err()
    );
    assert!(
        DirectoryPage::DirectoryBranch {
            format: 1,
            height: 0,
            pages: vec![p]
        }
        .validate()
        .is_err()
    );
}
#[test]
fn draft_modes_and_destructive_targets_are_explicit() {
    let mut value = json!({"root_id":"root","directory_id":"dir","expected_revision":"rev","base_version_id":null,"mode":"patch"});
    assert_eq!(
        serde_json::from_value::<DirectoryDraftBase>(value.clone())
            .unwrap()
            .mode,
        DraftMode::Patch
    );
    value["mode"] = "complete_replacement".into();
    assert_eq!(
        serde_json::from_value::<DirectoryDraftBase>(value.clone())
            .unwrap()
            .mode,
        DraftMode::CompleteReplacement
    );
    value.as_object_mut().unwrap().remove("mode");
    assert!(serde_json::from_value::<DirectoryDraftBase>(value).is_err());
    assert!(
        serde_json::from_value::<PurgeTarget>(json!({"kind":"file_path","path":"/file"})).is_err()
    );
    assert!(serde_json::from_value::<ApplyChangeRequest>(json!({"idempotency_key":"key","preview_id":"preview","expected_root_revision":"rev","acknowledge_retention_effect":false})).unwrap().validate().is_err());
    let bad:DraftMutationPage=serde_json::from_value(json!({"idempotency_key":"key","mutations":[{"kind":"upsert","identity":{"kind":"new","entry_id":"dir"},"parent_id":"dir","name":"loop","entry_kind":"directory","file":null}]})).unwrap();
    assert!(bad.validate().is_err());
}
