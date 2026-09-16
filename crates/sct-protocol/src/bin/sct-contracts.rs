use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mode = args
        .next()
        .ok_or("usage: sct-contracts <write|check> <repository-root>")?;
    let root = PathBuf::from(args.next().ok_or("repository root is required")?);
    if !matches!(mode.as_str(), "write" | "check") || args.next().is_some() {
        return Err("usage: sct-contracts <write|check> <repository-root>".into());
    }
    let artifacts = sct_protocol::codegen::artifacts()?;
    // This catches in-process nondeterminism without relying on Git state.
    if artifacts != sct_protocol::codegen::artifacts()? {
        return Err("nondeterministic contract generation".into());
    }
    let mut stale = Vec::new();
    for (name, contents) in artifacts {
        let destination = root.join(name);
        if mode == "write" {
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::write(&destination, contents)?;
        } else if fs::read_to_string(&destination).ok().as_ref() != Some(&contents) {
            stale.push(name);
        }
    }
    if !stale.is_empty() {
        return Err(format!(
            "contract drift: {}; run ./scripts/generate-contracts",
            stale.join(", ")
        )
        .into());
    }
    println!("Contracts {mode}: OK");
    Ok(())
}
