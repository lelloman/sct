use super::*;
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn graph_frontier_resumes_and_shared_tree_roots_protect_copies() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/graph-work", 4000).await;
    let fence = w
        .begin_operation(&a, &r.id, OperationKind::Upload)
        .await
        .unwrap();
    let memory = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([("one".into(), memory as Arc<dyn ObjectStore>)]);
    let e = TransferEngine::new(w.clone(), stores.clone(), 1, 60).unwrap();
    let file = FileManifest {
        format: 1,
        profile: CHUNK_PROFILE.into(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
        height: 0,
        tree: None,
    };
    let body = canonical(&file).unwrap();
    let shared = digest(&body);
    db::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,'file',0,0,$3)")
        .bind(r.id.as_str()).bind(&shared).bind(&body).execute(&db).await.unwrap();
    for i in 0..100 {
        let page = canonical(&DirectoryPage::DirectoryEntries {
            format: 1,
            entries: vec![DirectoryEntry {
                id: format!("entry-{i:03}").parse().unwrap(),
                name: "shared".parse().unwrap(),
                kind: EntryKind::File,
                manifest: shared.parse().unwrap(),
                length: 0.try_into().unwrap(),
                chunks: 0.try_into().unwrap(),
            }],
        })
        .unwrap();
        let page_id = digest(&page);
        let directory = canonical(&DirectoryManifest {
            format: 1,
            kind: DirectoryObjectKind::Directory,
            entries: 1.try_into().unwrap(),
            length: 0.try_into().unwrap(),
            chunks: 0.try_into().unwrap(),
            height: 0,
            tree: Some(page_id.parse().unwrap()),
        })
        .unwrap();
        let manifest = digest(&directory);
        for (id, kind, bytes) in [
            (&page_id, "directory_page", &page),
            (&manifest, "directory", &directory),
        ] {
            db::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,$3,0,0,$4)")
                .bind(r.id.as_str()).bind(id).bind(kind).bind(bytes).execute(&db).await.unwrap();
        }
        for (parent, child) in [(&manifest, &page_id), (&page_id, &shared)] {
            db::query("INSERT INTO content_edges(root_id,parent,ordinal,child,logical_length,chunk_count) VALUES($1,$2,0,$3,0,0)")
                .bind(r.id.as_str()).bind(parent).bind(child).execute(&db).await.unwrap();
        }
        db::query("INSERT INTO graph_pins(root_id,id,kind,manifest) VALUES($1,$2,'checkpoint',$3)")
            .bind(r.id.as_str())
            .bind(format!("pin-{i:03}"))
            .bind(manifest)
            .execute(&db)
            .await
            .unwrap();
    }
    let orphan = digest(b"orphan");
    db::query("INSERT INTO content_objects(root_id,digest,kind,length) VALUES($1,$2,'chunk',6)")
        .bind(r.id.as_str())
        .bind(&orphan)
        .execute(&db)
        .await
        .unwrap();
    let total = body.len() as u64 + 6;
    w.reserve_transfer(
        &fence,
        &BTreeMap::from([("pool".parse().unwrap(), total.try_into().unwrap())]),
    )
    .await
    .unwrap();
    e.write_copy(
        &fence,
        &shared.parse().unwrap(),
        &"one".parse().unwrap(),
        body.clone(),
    )
    .await
    .unwrap();
    e.write_copy(
        &fence,
        &orphan.parse().unwrap(),
        &"one".parse().unwrap(),
        b"orphan".to_vec(),
    )
    .await
    .unwrap();
    db::query("INSERT INTO transfer_sessions(id,root_id,operation_id,principal_id,key,request,manifest,state) VALUES('gc-session',$1,$2,$3,'gc','{}',$4,'aborted')")
        .bind(r.id.as_str()).bind(fence.id().as_str()).bind(a.principal_id()).bind(&shared).execute(&db).await.unwrap();
    e.tick().await.unwrap();
    assert_eq!(
        db::query_scalar::<_, String>("SELECT phase FROM graph_scans WHERE root_id=$1")
            .bind(r.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap(),
        "seeds"
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM object_copies WHERE state='available'")
            .fetch_one(&db)
            .await
            .unwrap(),
        2
    );
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let e = TransferEngine::new(w.clone(), stores, 1, 60).unwrap();
    for _ in 0..10 {
        e.tick().await.unwrap();
    }
    assert_eq!(
        db::query_scalar::<_, String>("SELECT state FROM object_copies WHERE digest=$1")
            .bind(&orphan)
            .fetch_one(&db)
            .await
            .unwrap(),
        "deleted"
    );
    assert_eq!(
        db::query_scalar::<_, String>("SELECT state FROM object_copies WHERE digest=$1")
            .bind(&shared)
            .fetch_one(&db)
            .await
            .unwrap(),
        "available"
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&db)
            .await
            .unwrap(),
        body.len() as i64
    );
    db::query("DELETE FROM graph_pins WHERE id<>'pin-099'")
        .execute(&db)
        .await
        .unwrap();
    for _ in 0..3 {
        e.tick().await.unwrap();
    }
    assert_eq!(
        db::query_scalar::<_, String>("SELECT state FROM object_copies WHERE digest=$1")
            .bind(&shared)
            .fetch_one(&db)
            .await
            .unwrap(),
        "available"
    );
    db::query("DELETE FROM graph_pins")
        .execute(&db)
        .await
        .unwrap();
    for _ in 0..3 {
        e.tick().await.unwrap();
    }
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM object_copies WHERE state<>'deleted'")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
}
