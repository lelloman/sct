//! Service-wide aggregate metrics: no principal, root, path, digest or generation labels.
use crate::{
    auth::{self, Actor},
    error::Result,
    lease::Writer,
};
use simple_server::database::postgres as db;

pub struct ServiceMetrics {
    pub root_states: Vec<(String, i64)>,
    pub operation_states: Vec<(String, i64)>,
    pub maintenance_states: Vec<(String, i64)>,
    pub checkpoint_states: Vec<(String, i64)>,
    pub freshness: Vec<(String, i64)>,
    pub backend_health: Vec<(String, i64)>,
    pub capacity: Vec<(&'static str, String)>,
}
impl Writer {
    pub async fn service_metrics(&self, actor: &Actor) -> Result<ServiceMetrics> {
        // Inspection must remain available during a checkpoint's write fence.
        let mut tx = self.control_transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let root_states = db::query_as("SELECT state,count(*)::bigint FROM roots GROUP BY state")
            .fetch_all(&mut *tx)
            .await?;
        let operation_states =
            db::query_as("SELECT state,count(*)::bigint FROM operations GROUP BY state")
                .fetch_all(&mut *tx)
                .await?;
        let maintenance_states =
            db::query_as("SELECT state,count(*)::bigint FROM maintenance_jobs GROUP BY state")
                .fetch_all(&mut *tx)
                .await?;
        let checkpoint_states =
            db::query_as("SELECT state,count(*)::bigint FROM checkpoints GROUP BY state")
                .fetch_all(&mut *tx)
                .await?;
        let freshness = db::query_as("SELECT CASE WHEN expected_upload_interval_seconds IS NULL THEN 'not_configured' WHEN last_commit_at IS NULL THEN 'never_committed' WHEN last_commit_at + expected_upload_interval_seconds * interval '1 second' < clock_timestamp() THEN 'overdue' ELSE 'on_time' END AS freshness,count(*)::bigint FROM roots GROUP BY freshness").fetch_all(&mut *tx).await?;
        let backend_health = db::query_as("SELECT COALESCE(inspection->>'observed_health','unknown'),count(*)::bigint FROM backends GROUP BY 1").fetch_all(&mut *tx).await?;
        let row = db::query("SELECT COALESCE(sum(capacity),0)::text AS capacity,COALESCE(sum(reserved),0)::text AS reserved,COALESCE(sum(occupied),0)::text AS occupied,COALESCE(sum(pending_cleanup),0)::text AS pending_cleanup FROM allocations").fetch_one(&mut *tx).await?;
        let capacity = ["capacity", "reserved", "occupied", "pending_cleanup"]
            .map(|name| (name, row.get::<String, _>(name)))
            .to_vec();
        Ok(ServiceMetrics {
            root_states,
            operation_states,
            maintenance_states,
            checkpoint_states,
            freshness,
            backend_health,
            capacity,
        })
    }
}
