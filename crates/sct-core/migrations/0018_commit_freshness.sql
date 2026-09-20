-- Only durable publications establish backup freshness. Preserve successful
-- commits made before this feature; admission/upload attempts are not backups.
UPDATE roots r SET last_commit_at = p.committed_at
FROM (SELECT root_id,max(created_at) AS committed_at FROM transfer_audit
      WHERE event='publication' GROUP BY root_id) p
WHERE r.id=p.root_id AND r.last_commit_at IS NULL;
