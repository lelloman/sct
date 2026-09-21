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
    if args.len() != 3 || !matches!(args[0].to_str(), Some("inspect" | "verify")) {
        eprintln!(
            "Usage: sct-recover inspect|verify DISC_DIRECTORY PRIVATE_WORK.sqlite\nReads mounted or extracted discs offline. Verification emits JSONL observations and a summary. Exit 2 means some payloads could not be verified."
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
    let result = index.verify_media(media, &mut out).await?;
    serde_json::to_writer(&mut out, &serde_json::json!({"summary":result}))?;
    out.write_all(b"\n")?;
    eprintln!(
        "Disc {}: {} verified, {} unavailable objects",
        result.disc_id, result.verified_objects, result.unavailable_objects
    );
    Ok(if result.complete() { 0 } else { 2 })
}
