use sct_archive::*;
use std::{io::Write, path::Path};

#[tokio::main]
async fn main() {
    match run().await {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("sct-recover: {error}");
            std::process::exit(1);
        }
    }
}
async fn run() -> Result<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 2 && matches!(args[0].to_str(), Some("catalog-inspect" | "catalog-export")) {
        let database = Path::new(&args[1]);
        ensure(database.exists(), "catalog does not exist")?;
        ArchiveCatalog::open(database)
            .await?
            .export(&mut std::io::stdout(), args[0] == "catalog-export")
            .await?;
        return Ok(0);
    }
    if args.len() == 4 && args[0] == "catalog-scan" {
        let media = Path::new(&args[1]);
        let work = Path::new(&args[3]);
        let index = if work.exists() {
            ArchiveIndex::open(work).await?
        } else {
            ArchiveIndex::create(media, work).await?
        };
        ArchiveCatalog::open(Path::new(&args[2]))
            .await?
            .scan(media, &index)
            .await?;
        return Ok(0);
    }
    if args.first().is_some_and(|v| v == "restore")
        && (args.len() == 4 || (args.len() == 5 && args[4] == "--partial"))
    {
        let mut index = ArchiveIndex::open(Path::new(&args[1])).await?;
        let result = index
            .restore_to(
                Path::new(&args[2]),
                Path::new(&args[3]),
                args.len() == 5,
                &mut std::io::stdout(),
            )
            .await?;
        return Ok(if result.complete { 0 } else { 2 });
    }
    if args.first().is_some_and(|v| v == "ingest") && args.len() == 4 {
        let media = Path::new(&args[1]);
        let database = Path::new(&args[2]);
        let mut index = if database.exists() {
            ArchiveIndex::open(database).await?
        } else {
            ArchiveIndex::create(media, database).await?
        };
        let result = index
            .ingest(media, Path::new(&args[3]), &mut std::io::stdout())
            .await?;
        return Ok(if result.complete() { 0 } else { 2 });
    }
    if args.len() != 3
        || !matches!(
            args[0].to_str(),
            Some("inspect" | "verify" | "verify-report")
        )
    {
        eprintln!(
            "Usage: sct-recover inspect|verify|verify-report DISC_DIRECTORY PRIVATE_WORK.sqlite\n       sct-recover ingest DISC_DIRECTORY PRIVATE_WORK.sqlite PRIVATE_CACHE_DIRECTORY\n       sct-recover restore PRIVATE_WORK.sqlite PRIVATE_CACHE_DIRECTORY DESTINATION [--partial]\n       sct-recover catalog-scan DISC_DIRECTORY PRIVATE_CATALOG.sqlite PRIVATE_WORK.sqlite\n       sct-recover catalog-inspect|catalog-export PRIVATE_CATALOG.sqlite\nReads mounted or extracted discs offline. Verification emits JSONL observations and a summary. Exit 2 means some payloads could not be verified."
        );
        return Ok(64);
    }
    let media = Path::new(&args[1]);
    let database = Path::new(&args[2]);
    let mut index = if database.exists() {
        let index = ArchiveIndex::open(database).await?;
        let disc = descriptor(&SafeDir::open(media)?)?;
        ensure(
            disc.collection_id == index.descriptor.collection_id
                && disc.index_sha256 == index.descriptor.index_sha256,
            "work index belongs to a different snapshot; select a different private work file",
        )?;
        index
    } else {
        ArchiveIndex::create(media, database).await?
    };
    let mut out = std::io::stdout();
    if args[0] == "inspect" {
        index.inspect(&mut out).await?;
        return Ok(0);
    }
    if args[0] == "verify-report" {
        let report = index
            .verify_disc_report(media, &mut std::io::stderr())
            .await?;
        let complete =
            report.metadata_verified && report.expected_objects == report.verified_objects;
        serde_json::to_writer(&mut out, &report)?;
        out.write_all(b"\n")?;
        return Ok(if complete { 0 } else { 2 });
    }
    let result = index.verify_media(media, &mut out).await?;
    serde_json::to_writer(&mut out, &serde_json::json!({"summary":result}))?;
    out.write_all(b"\n")?;
    eprintln!(
        "Disc {}: {} verified, {} unavailable objects",
        result.disc_id, result.verified_objects, result.unavailable_objects
    );
    Ok(if result.complete() { 0 } else { 2 })
}
