// Compile-only checks: identities and mutation preconditions remain distinct.
import type {
  EntryId,
  EntryName,
  RootId,
  ManifestId,
  VersionId,
  RenameEntryRequest,
  Placement,
} from "../src/generated/api";
function identityChecks(
  root: RootId,
  entry: EntryId,
  name: EntryName,
  manifest: ManifestId,
  version: VersionId,
) {
  // @ts-expect-error Root IDs are not entry IDs.
  const wrongEntry: EntryId = root;
  // @ts-expect-error Manifest digests are not version IDs.
  const wrongVersion: VersionId = manifest;
  // @ts-expect-error Mutations require expected revisions.
  const incomplete: RenameEntryRequest = { destination_parent_id: entry, name };
  const invalid = {
    kind: "mirror" as const,
    backend_ids: [] as [],
    read_policy: { candidates: [], overall_deadline_ms: null },
  };
  // @ts-expect-error A mirror must contain exactly two backend IDs.
  const wrongMirror: Placement = invalid;
  return [entry, version, wrongEntry, wrongVersion, incomplete, wrongMirror];
}
void identityChecks;
