//! SCT_ORIGIN=http://localhost:8080 SCT_TOKEN=<token> cargo run -p sct-client --example upload -- ROOT SOURCE JOB_DIRECTORY
use sct_client::{Client, UploadJob, protocol::*};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected ROOT SOURCE JOB_DIRECTORY".into());
    }
    let root: RootId = args[1].parse()?;
    let source = std::path::Path::new(&args[2]);
    let directory = std::path::Path::new(&args[3]);
    let client = Client::new(std::env::var("SCT_ORIGIN")?.parse()?)?
        .with_token(std::env::var("SCT_TOKEN")?)?;
    let job = if directory.exists() {
        UploadJob::open(directory)?
    } else {
        let parent = client.resolve_path(&root, &"/".parse()?).await?;
        let policy = client
            .effective_policy(&PolicyScope::Root {
                root_id: root.clone(),
            })
            .await?;
        // Persisted in the job; never generate a new key when retrying that job.
        let key = format!(
            "upload-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        )
        .parse()?;
        UploadJob::prepare(
            source,
            directory,
            FileTarget::Create {
                parent_id: parent.id,
                expected_parent_revision: parent.revision,
                name: source
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("invalid filename")?
                    .parse()?,
            },
            policy.revision,
            key,
        )?
    };
    let receipt = client.upload(&root, &job).await?;
    println!("{}", serde_json::to_string(&receipt)?);
    Ok(())
}
