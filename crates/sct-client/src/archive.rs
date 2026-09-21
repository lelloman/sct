//! Typed archive contracts. Routes are advertised only when the server implements them.
use crate::{Client, Error, protocol::*};
use reqwest::Method;
impl Client {
    pub async fn list_archive_entries(
        &self,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ListRequest,
    ) -> Result<ArchiveEntryPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/archives/{}/sets/{}/entries",
                collection.as_str(),
                set.as_str()
            ),
            archive_page_query(request),
            None,
        )
        .await
    }
    pub async fn list_archive_objects(
        &self,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        request: &ListRequest,
    ) -> Result<ArchiveObjectPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/archives/{}/discs/{}/objects",
                collection.as_str(),
                disc.as_str()
            ),
            archive_page_query(request),
            None,
        )
        .await
    }

    pub async fn list_archive_collections(
        &self,
        request: &ListRequest,
    ) -> Result<ArchiveCollectionPage, Error> {
        self.request(
            Method::GET,
            "/api/v1/archives",
            archive_page_query(request),
            None,
        )
        .await
    }
    pub async fn create_archive_collection(
        &self,
        request: &CreateArchiveCollectionRequest,
    ) -> Result<ArchiveCollection, Error> {
        self.request(
            Method::POST,
            "/api/v1/archives",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn inspect_archive_collection(
        &self,
        collection: &ArchiveCollectionId,
    ) -> Result<ArchiveCollection, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/archives/{}", collection.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn update_archive_collection(
        &self,
        collection: &ArchiveCollectionId,
        request: &UpdateArchiveCollectionRequest,
    ) -> Result<ArchiveCollection, Error> {
        self.request(
            Method::PATCH,
            &format!("/api/v1/archives/{}", collection.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn archive_usage(&self) -> Result<ArchiveUsage, Error> {
        self.request(Method::GET, "/api/v1/archives/usage", vec![], None)
            .await
    }
    pub async fn create_archive_plan(
        &self,
        collection: &ArchiveCollectionId,
        request: &CreateArchivePlanRequest,
    ) -> Result<ArchiveSet, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/archives/{}/sets", collection.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_archive_sets(
        &self,
        collection: &ArchiveCollectionId,
        request: &ListRequest,
    ) -> Result<ArchiveSetPage, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/archives/{}/sets", collection.as_str()),
            archive_page_query(request),
            None,
        )
        .await
    }
    pub async fn inspect_archive_set(
        &self,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
    ) -> Result<ArchiveSet, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/archives/{}/sets/{}",
                collection.as_str(),
                set.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn confirm_archive_set(
        &self,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ArchiveSetActionRequest,
    ) -> Result<ArchiveSet, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/archives/{}/sets/{}/confirm",
                collection.as_str(),
                set.as_str()
            ),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn cancel_archive_set(
        &self,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ArchiveSetActionRequest,
    ) -> Result<ArchiveSet, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/archives/{}/sets/{}/cancel",
                collection.as_str(),
                set.as_str()
            ),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_archive_discs(
        &self,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ListRequest,
    ) -> Result<ArchiveDiscPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/archives/{}/sets/{}/discs",
                collection.as_str(),
                set.as_str()
            ),
            archive_page_query(request),
            None,
        )
        .await
    }
    pub async fn list_archive_copies(
        &self,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        request: &ListRequest,
    ) -> Result<ArchiveCopyPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/archives/{}/discs/{}/copies",
                collection.as_str(),
                disc.as_str()
            ),
            archive_page_query(request),
            None,
        )
        .await
    }
    pub async fn register_archive_copy(
        &self,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        request: &RegisterArchiveCopyRequest,
    ) -> Result<ArchivePhysicalCopy, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/archives/{}/discs/{}/copies",
                collection.as_str(),
                disc.as_str()
            ),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn update_archive_copy(
        &self,
        collection: &ArchiveCollectionId,
        copy: &ArchiveCopyId,
        request: &UpdateArchiveCopyRequest,
    ) -> Result<ArchivePhysicalCopy, Error> {
        self.request(
            Method::PATCH,
            &format!(
                "/api/v1/archives/{}/copies/{}",
                collection.as_str(),
                copy.as_str()
            ),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn submit_archive_verification(
        &self,
        collection: &ArchiveCollectionId,
        copy: &ArchiveCopyId,
        request: &SubmitArchiveVerificationRequest,
    ) -> Result<ArchiveVerificationReport, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/archives/{}/copies/{}/reports",
                collection.as_str(),
                copy.as_str()
            ),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_archive_reports(
        &self,
        collection: &ArchiveCollectionId,
        copy: &ArchiveCopyId,
        request: &ListRequest,
    ) -> Result<ArchiveReportPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/archives/{}/copies/{}/reports",
                collection.as_str(),
                copy.as_str()
            ),
            archive_page_query(request),
            None,
        )
        .await
    }
}
fn archive_page_query(request: &ListRequest) -> Vec<(&'static str, String)> {
    let mut query = vec![("limit", request.limit.get().to_string())];
    if let Some(cursor) = &request.cursor {
        query.push(("cursor", cursor.as_str().into()));
    }
    query
}

impl Client {
    /// Stream to caller-owned temporary output; promote only after length and digest
    /// verification succeeds. Download completion is not a physical burn observation.
    pub async fn download_archive_iso_to<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        artifact: &ArchiveArtifact,
        output: &mut W,
    ) -> Result<u64, Error> {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncWriteExt;
        if artifact.state != ArchiveArtifactState::Available {
            return Err(Error::Local("archive ISO is not available".into()));
        }
        let length = artifact.length.as_ref().ok_or(Error::Integrity)?.get();
        let expected = artifact.sha256.as_ref().ok_or(Error::Integrity)?;
        let mut response = self
            .request_response(
                Method::GET,
                &format!(
                    "/api/v1/archives/{}/discs/{}/iso",
                    collection.as_str(),
                    disc.as_str()
                ),
                vec![],
                None,
            )
            .await?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::UnexpectedStatus(response.status().as_u16()));
        }
        if response.content_length() != Some(length) {
            return Err(Error::Integrity);
        }
        let mut count = 0u64;
        let mut hash = Sha256::new();
        while let Some(chunk) = response.chunk().await.map_err(Error::Transport)? {
            count = count
                .checked_add(chunk.len() as u64)
                .ok_or(Error::Integrity)?;
            if count > length {
                return Err(Error::Integrity);
            }
            hash.update(&chunk);
            output.write_all(&chunk).await.map_err(Error::Io)?;
        }
        if count != length || format!("{:x}", hash.finalize()) != expected.as_str() {
            return Err(Error::Integrity);
        }
        output.flush().await.map_err(Error::Io)?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn iso_download_checks_the_selected_immutable_artifact() {
        for (body, expected_success) in [(b"abc".as_slice(), true), (b"bad", false), (b"ab", false)]
        {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}/", listener.local_addr().unwrap());
            let body = body.to_vec();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 8192);
                }
                assert!(
                    request
                        .starts_with(b"GET /api/v1/archives/archive-1/discs/disc-1/iso HTTP/1.1")
                );
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                socket.write_all(&body).await.unwrap();
            });
            let client = Client::new(origin.parse().unwrap()).unwrap();
            let artifact = ArchiveArtifact {
                id: "image".parse().unwrap(),
                state: ArchiveArtifactState::Available,
                sha256: Some(digest(b"abc").parse().unwrap()),
                length: Some(3.try_into().unwrap()),
                expires_at_ms: 100.try_into().unwrap(),
                download_count: 0.try_into().unwrap(),
            };
            let mut output = Vec::new();
            let result = client
                .download_archive_iso_to(
                    &"archive-1".parse().unwrap(),
                    &"disc-1".parse().unwrap(),
                    &artifact,
                    &mut output,
                )
                .await;
            assert_eq!(result.is_ok(), expected_success);
            if expected_success {
                assert_eq!(output, b"abc");
            }
            server.await.unwrap();
        }
    }
}
