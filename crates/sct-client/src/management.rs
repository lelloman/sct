//! Typed management calls. Callers own scheduling, retry decisions and recovery qualification.
use crate::{Client, Error, protocol::*};
use reqwest::Method;

fn page_query(request: &ListRequest) -> Vec<(&'static str, String)> {
    let mut query = vec![("limit", request.limit.get().to_string())];
    if let Some(cursor) = &request.cursor {
        query.push(("cursor", cursor.as_str().into()));
    }
    query
}
impl Client {
    pub async fn account(&self) -> Result<AccountInspection, Error> {
        self.request(Method::GET, "/api/v1/account", vec![], None)
            .await
    }
    pub async fn list_principals(&self, request: &ListRequest) -> Result<PrincipalPage, Error> {
        self.request(Method::GET, "/api/v1/principals", page_query(request), None)
            .await
    }
    pub async fn create_principal(
        &self,
        request: &CreatePrincipalRequest,
    ) -> Result<PrincipalInspection, Error> {
        self.request(
            Method::POST,
            "/api/v1/principals",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn inspect_principal(
        &self,
        principal: &PrincipalId,
    ) -> Result<PrincipalInspection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/principals/{}", principal.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn update_principal(
        &self,
        principal: &PrincipalId,
        request: &UpdatePrincipalRequest,
    ) -> Result<PrincipalInspection, Error> {
        self.request(
            Method::PATCH,
            &format!("/api/v1/principals/{}", principal.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_credentials(
        &self,
        principal: &PrincipalId,
        request: &ListRequest,
    ) -> Result<CredentialPage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/principals/{}/credentials", principal.as_str()),
            page_query(request),
            None,
        )
        .await
    }
    pub async fn issue_credential(&self, principal: &PrincipalId) -> Result<IssuedToken, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/principals/{}/credentials", principal.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn root_management(&self, root: &RootId) -> Result<RootManagement, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/management", root.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn update_root_management(
        &self,
        root: &RootId,
        request: &UpdateRootManagementRequest,
    ) -> Result<RootManagement, Error> {
        self.request(
            Method::PATCH,
            &format!("/api/v1/roots/{}/management", root.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn update_quota_ceiling(
        &self,
        root: &RootId,
        request: &UpdateQuotaCeilingRequest,
    ) -> Result<RootManagement, Error> {
        self.request(
            Method::PATCH,
            &format!("/api/v1/roots/{}/quota-ceiling", root.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn set_policy_constraints(
        &self,
        root: &RootId,
        request: &PolicyConstraintsRequest,
    ) -> Result<RootInspection, Error> {
        self.request(
            Method::PATCH,
            &format!("/api/v1/admin/roots/{}/policy-constraints", root.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn root_operation(&self, root: &RootId) -> Result<OperationInspection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/operation", root.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn list_grants(
        &self,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<GrantPage, Error> {
        self.request(
            Method::GET,
            "/api/v1/admin/grants",
            {
                let mut q = page_query(request);
                q.push(("root_id", root.as_str().into()));
                q
            },
            None,
        )
        .await
    }
    pub async fn inspect_grant(&self, grant: &GrantId) -> Result<GrantInspection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/admin/grants/{}", grant.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn create_grant(
        &self,
        request: &CreateGrantRequest,
    ) -> Result<GrantInspection, Error> {
        self.request(
            Method::POST,
            "/api/v1/admin/grants",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn update_grant(
        &self,
        grant: &GrantId,
        request: &UpdateGrantRequest,
    ) -> Result<GrantInspection, Error> {
        self.request(
            Method::PATCH,
            &format!("/api/v1/admin/grants/{}", grant.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_backends(&self, request: &ListRequest) -> Result<BackendPage, Error> {
        self.request(
            Method::GET,
            "/api/v1/admin/backends",
            page_query(request),
            None,
        )
        .await
    }
    pub async fn inspect_backend(&self, backend: &BackendId) -> Result<BackendInspection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/admin/backends/{}", backend.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn backend_configuration(
        &self,
        backend: &BackendId,
    ) -> Result<BackendConfiguration, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/admin/backends/{}/configuration", backend.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn configure_backend(
        &self,
        backend: &BackendId,
        request: &ConfigureBackendRequest,
    ) -> Result<BackendConfiguration, Error> {
        self.request(
            Method::PUT,
            &format!("/api/v1/admin/backends/{}", backend.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_pools(&self, request: &ListRequest) -> Result<PoolPage, Error> {
        self.request(
            Method::GET,
            "/api/v1/admin/pools",
            page_query(request),
            None,
        )
        .await
    }
    pub async fn inspect_pool(&self, pool: &PoolId) -> Result<PoolInspection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/admin/pools/{}", pool.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn create_pool(&self, request: &CreatePoolRequest) -> Result<PoolInspection, Error> {
        self.request(
            Method::POST,
            "/api/v1/admin/pools",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn update_pool(
        &self,
        pool: &PoolId,
        request: &UpdatePoolRequest,
    ) -> Result<PoolInspection, Error> {
        self.request(
            Method::PATCH,
            &format!("/api/v1/admin/pools/{}", pool.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_allocations(&self, request: &ListRequest) -> Result<AllocationPage, Error> {
        self.request(
            Method::GET,
            "/api/v1/admin/allocations",
            page_query(request),
            None,
        )
        .await
    }
    pub async fn inspect_allocation(
        &self,
        allocation: &AllocationId,
    ) -> Result<AllocationInspection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/admin/allocations/{}", allocation.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn allocation_management(
        &self,
        allocation: &AllocationId,
    ) -> Result<AllocationManagement, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/admin/allocations/{}/management",
                allocation.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn create_allocation(
        &self,
        request: &CreateAllocationRequest,
    ) -> Result<AllocationManagement, Error> {
        self.request(
            Method::POST,
            "/api/v1/admin/allocations",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn resize_allocation(
        &self,
        allocation: &AllocationId,
        request: &ResizeAllocationRequest,
    ) -> Result<AllocationManagement, Error> {
        self.request(
            Method::PUT,
            &format!("/api/v1/admin/allocations/{}", allocation.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn root_usage(&self, root: &RootId) -> Result<RootCapacityUsage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/usage", root.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn list_transfer_activity(
        &self,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<TransferActivityPage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/sessions", root.as_str()),
            page_query(request),
            None,
        )
        .await
    }
    pub async fn list_root_jobs(
        &self,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<JobActivityPage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/jobs", root.as_str()),
            page_query(request),
            None,
        )
        .await
    }
    pub async fn list_root_audit(
        &self,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<AuditPage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/audit", root.as_str()),
            page_query(request),
            None,
        )
        .await
    }
    pub async fn list_jobs(&self, request: &ListRequest) -> Result<JobActivityPage, Error> {
        self.request(Method::GET, "/api/v1/admin/jobs", page_query(request), None)
            .await
    }
    pub async fn list_audit(&self, request: &ListRequest) -> Result<AuditPage, Error> {
        self.request(
            Method::GET,
            "/api/v1/admin/audit",
            page_query(request),
            None,
        )
        .await
    }
    pub async fn start_integrity_scan(
        &self,
        request: &StartMaintenanceRequest,
    ) -> Result<MaintenanceJob, Error> {
        self.request(
            Method::POST,
            "/api/v1/admin/maintenance",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn start_repair(
        &self,
        root: &RootId,
        request: &StartMaintenanceRequest,
    ) -> Result<MaintenanceJob, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/admin/roots/{}/repair", root.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_maintenance(&self, request: &ListRequest) -> Result<MaintenancePage, Error> {
        self.request(
            Method::GET,
            "/api/v1/admin/maintenance",
            page_query(request),
            None,
        )
        .await
    }
    pub async fn list_root_maintenance(
        &self,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<MaintenancePage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/maintenance", root.as_str()),
            page_query(request),
            None,
        )
        .await
    }
    pub async fn maintenance_job(
        &self,
        root: &RootId,
        job: &OperationId,
    ) -> Result<MaintenanceJob, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/maintenance/{}",
                root.as_str(),
                job.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn resume_maintenance(
        &self,
        job: &OperationId,
        request: &ExpectedRevision,
    ) -> Result<MaintenanceJob, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/admin/maintenance/{}/resume", job.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_integrity_findings(
        &self,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<IntegrityFindingPage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/integrity-findings", root.as_str()),
            page_query(request),
            None,
        )
        .await
    }
    pub async fn create_checkpoint(
        &self,
        request: &CreateCheckpointRequest,
    ) -> Result<CheckpointInspection, Error> {
        self.request(
            Method::POST,
            "/api/v1/admin/checkpoints",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_checkpoints(&self, request: &ListRequest) -> Result<CheckpointPage, Error> {
        self.request(
            Method::GET,
            "/api/v1/admin/checkpoints",
            page_query(request),
            None,
        )
        .await
    }
    pub async fn checkpoint(
        &self,
        checkpoint: &OperationId,
    ) -> Result<CheckpointInspection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/admin/checkpoints/{}", checkpoint.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn release_checkpoint(
        &self,
        checkpoint: &OperationId,
        request: &ReleaseCheckpointRequest,
    ) -> Result<CheckpointInspection, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/admin/checkpoints/{}/abandon", checkpoint.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn checkpoint_inventory(
        &self,
        checkpoint: &OperationId,
        request: &ListRequest,
    ) -> Result<CheckpointInventoryPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/admin/checkpoints/{}/inventory",
                checkpoint.as_str()
            ),
            page_query(request),
            None,
        )
        .await
    }
    pub async fn checkpoint_descriptor(
        &self,
        checkpoint: &OperationId,
    ) -> Result<CheckpointDescriptor, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/admin/checkpoints/{}/descriptor",
                checkpoint.as_str()
            ),
            vec![],
            None,
        )
        .await
    }

    /// Revoke immediately; a lost response can be reconciled through credential inspection.
    pub async fn revoke_credential(&self, credential: &CredentialId) -> Result<(), Error> {
        self.request_empty(
            Method::DELETE,
            &format!("/api/v1/credentials/{}", credential.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn revoke_grant(&self, grant: &GrantId, expected: &Revision) -> Result<(), Error> {
        self.request_empty(
            Method::DELETE,
            &format!("/api/v1/admin/grants/{}", grant.as_str()),
            vec![("expected_revision", expected.as_str().into())],
            None,
        )
        .await
    }
    pub async fn adapter_documentation(&self, kind: &str) -> Result<AdapterDocumentation, Error> {
        if kind.is_empty()
            || kind.len() > 128
            || !kind
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::Local("invalid adapter kind".into()));
        }
        self.request(
            Method::GET,
            &format!("/api/v1/admin/adapters/{kind}"),
            vec![],
            None,
        )
        .await
    }
    /// Readiness includes valid writer ownership and the catalog write gate.
    pub async fn check_readiness(&self) -> Result<(), Error> {
        self.request_empty(Method::GET, "/health/ready", vec![], None)
            .await
    }
}

/// Only these fixed artifacts are downloadable; arbitrary service paths are not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointArtifact {
    Catalog,
    Inventory,
    Descriptor,
}
impl CheckpointArtifact {
    fn filename(self) -> &'static str {
        match self {
            Self::Catalog => "catalog.jsonl",
            Self::Inventory => "inventory.jsonl",
            Self::Descriptor => "descriptor.json",
        }
    }
}
impl Client {
    /// Stream into caller-owned temporary output. Promote it only after success.
    /// Catalog/inventory hashes must match the exported descriptor. Downloading
    /// artifacts does not copy payloads or prove independent recoverability.
    pub async fn download_checkpoint_artifact_to<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        checkpoint: &OperationId,
        artifact: CheckpointArtifact,
        output: &mut W,
    ) -> Result<u64, Error> {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncWriteExt;
        let descriptor = self.checkpoint_descriptor(checkpoint).await?;
        let expected = match artifact {
            CheckpointArtifact::Catalog => Some(&descriptor.catalog_digest),
            CheckpointArtifact::Inventory => Some(&descriptor.inventory_digest),
            CheckpointArtifact::Descriptor => None,
        };
        let mut response = self
            .request_response(
                Method::GET,
                &format!(
                    "/api/v1/admin/checkpoints/{}/artifacts/{}",
                    checkpoint.as_str(),
                    artifact.filename()
                ),
                vec![],
                None,
            )
            .await?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::UnexpectedStatus(response.status().as_u16()));
        }
        let mut hash = Sha256::new();
        let mut count = 0u64;
        let mut small = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Error::Transport)? {
            count = count
                .checked_add(chunk.len() as u64)
                .ok_or(Error::Integrity)?;
            if artifact == CheckpointArtifact::Descriptor {
                if count > 65536 {
                    return Err(Error::Integrity);
                }
                small.extend_from_slice(&chunk);
            }
            hash.update(&chunk);
            output.write_all(&chunk).await.map_err(Error::Io)?;
        }
        if let Some(expected) = expected {
            if format!("{:x}", hash.finalize()) != expected.as_str() {
                return Err(Error::Integrity);
            }
        } else if serde_json::from_slice::<CheckpointDescriptor>(&small)? != descriptor {
            return Err(Error::Integrity);
        }
        output.flush().await.map_err(Error::Io)?;
        Ok(count)
    }
}
