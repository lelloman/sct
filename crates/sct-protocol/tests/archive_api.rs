use sct_protocol::*;
#[test]
fn selections_reject_ambiguous_composition_and_bound_work() {
    let mut request = CreateArchivePlanRequest {
        request_id: "request".parse().unwrap(),
        expected_revision: "r1".parse().unwrap(),
        media_capacity_bytes: 25_000_000_000.try_into().unwrap(),
        selections: vec![],
    };
    assert!(request.validate().is_err());
    for path in ["/a", "/a-", "/a/b"] {
        request.selections.push(ArchiveSelection {
            root_id: "root".parse().unwrap(),
            version_id: "version".parse().unwrap(),
            path: path.parse().unwrap(),
        });
    }
    assert!(request.validate().is_err());
    request.selections.pop();
    assert!(request.validate().is_ok());
    request.selections = vec![request.selections[0].clone(); 1025];
    assert!(request.validate().is_err());
}
#[test]
fn published_history_and_observations_do_not_collapse_into_one_state() {
    use ArchiveJobState::*;
    assert!(Building.can_transition_to(&Ready));
    assert!(Building.can_transition_to(&Revoked));
    assert!(!Ready.can_transition_to(&Expired));
    assert!(!Cancelled.can_transition_to(&Building));
    assert!(Ready.terminal());
    let update = UpdateArchiveCopyRequest {
        expected_revision: "r1".parse().unwrap(),
        label: "disc shelf 1".parse().unwrap(),
        health: Some(ArchiveCopyHealth::ReadBackVerified),
        location: String::new(),
        reported_burned_at_ms: None,
    };
    assert!(update.validate().is_err());
    assert_eq!(ErrorCode::ArchiveArtifactUnavailable.http_status(), 410);
    assert_eq!(ErrorCode::ArchiveMetadataTooLarge.http_status(), 422);
}
