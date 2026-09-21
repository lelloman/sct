# Recovering archive metadata

Every disc contains its set's complete target snapshot index and disc map. The
standalone reader can collect these indexes without the service, PostgreSQL,
a login, or network access:

```sh
sct-recover catalog-scan /media/disc-5 /private/catalog.sqlite /private/set-2.sqlite
sct-recover catalog-scan /media/disc-1 /private/catalog.sqlite /private/set-1.sqlite
sct-recover catalog-inspect /private/catalog.sqlite
sct-recover catalog-export /private/catalog.sqlite > /private/catalog.jsonl
```

Use a private writable directory for the catalog and work files. Each work file
belongs to one snapshot. Scanning any representative disc from a set records its
entire index, including dependencies on earlier discs. Scanning representatives
from other sets adds separate historical recovery points. Scanning a second disc
from the same set records that its metadata was read without duplicating the
snapshot. Conflicting identities, indexes, disc numbers, object lengths, or known
original payload inventories are rejected atomically.

`catalog-inspect` emits a JSONL inventory; `catalog-export` also emits every
original index record, in its original order, wrapped with collection and set
identities. Both stream bounded pages from the disk-backed SQLite catalog.
These are metadata exports, not replacement recovery media: preserve the discs,
their recovery software, and payloads too. Use `ingest` and `restore` for file
recovery, and `verify-report` for full original-disc read-back.

The export explicitly reports partial knowledge. A reference to disc 1 from
disc 5 does not prove disc 1 was read, recover disc 1's original snapshot index,
or establish that a physical copy is healthy. Unknown earlier indexes remain
unknown until scanned. Unknown later sets may always exist. Disc metadata cannot
recover service accounts, grants, credentials, physical-copy observations, or
server ownership. Collections remain distinct by their recorded identities;
there is no automatic import into an unrelated account.

## Administrator checkpoint recovery

Service checkpoints include archive collections, recovery points, immutable
indexes and placements, image hashes and lengths, physical copies, read-back
observations, and event history. Restoring a checkpoint preserves those records
and reserved disc numbers. It fails unfinished archive jobs, fences their worker
epochs, releases their pins and reservations, clears download leases, and marks
previously available cached images expired. Old staging/cache paths are never
opened or deleted by checkpoint recovery. Configure fresh local export storage
on the recovered service.

Recovered collections require reconciliation and cannot accept new appends in
this version. A checkpoint can predate subsequent burns; neither its saved
highest number nor the highest supplied disc proves that no later discs exist.
Create a new collection for future exports, retaining the recovered collection
as permanent historical inventory. This prevents number/identity reuse and
never silently maps source identities onto another account.
