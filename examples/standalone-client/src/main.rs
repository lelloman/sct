use sct_client::{Client, Error, protocol::ListRequest};

// This fixture is compiled outside the repository from packaged crates. It
// deliberately does not connect during the package check.
async fn inspect_management(client: &Client) -> Result<(), Error> {
    let page = ListRequest {
        limit: 10.try_into().unwrap(),
        cursor: None,
    };
    let account = client.account().await?;
    for collection in client.list_archive_collections(&page).await?.collections {
        let sets = client.list_archive_sets(&collection.id, &page).await?;
        for set in sets.sets {
            client.list_archive_discs(&collection.id, &set.id, &page).await?;
        }
    }
    let roots = client.list_roots(&page).await?;
    for root in roots.items {
        let management = client.root_management(&root.id).await?;
        let usage = client.root_usage(&root.id).await?;
        println!(
            "{}: {:?}, {} retained bytes",
            root.display_name.as_str(),
            management.backup_freshness,
            usage.unique_retained_bytes.get()
        );
    }
    client
        .list_credentials(&account.principal_id, &page)
        .await?;
    if account.administrator {
        client.list_pools(&page).await?;
        client.list_backends(&page).await?;
        client.list_allocations(&page).await?;
        client.list_maintenance(&page).await?;
        client.list_checkpoints(&page).await?;
    }
    Ok(())
}
fn main() {
    let client = Client::new("http://localhost:8080".parse().unwrap()).unwrap();
    drop(inspect_management(&client));
    println!("ScT public API: {}", sct_client::protocol::API_PREFIX);
}
