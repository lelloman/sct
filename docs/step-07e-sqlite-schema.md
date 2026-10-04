# Step 07e: offline archive SQLite creation

Reviewed 2026-10-04 on active `master`, starting at
`77732bae83d9b6a9d63761ece6b7a071970427f9`. The two production creation paths
in `sct-archive` now call shared creation plans: `ArchiveIndex::create` for its
private disk-backed work index and `ArchiveCatalog::open` for a new offline
catalog. All six index tables/one explicit index and six catalog tables/two
explicit indexes are described locally through `database::sqlite::schema`.
Existing foreign keys, nullable metadata, composite primary/unique keys,
column ordering and BINARY ascending index terms are retained.

Catalog creation uses the shared 0.1.2 column CHECK description for
`catalog_meta.format=1`. Its original ordering is preserved: create the header,
insert format marker 1, create the remaining tables/indexes, commit the existing
SQLx transaction. Index creation remains inside the transaction which also
validates/imports the archive graph; unusable media cannot commit a valid index.
No IF NOT EXISTS, repair, implicit transaction, PRAGMA or user_version writes
were added. Archive media format, file creation modes, advisory locking,
symlink protections, disk-backed storage, cache/temp-store/journal choices and
restore/query ownership remain application-controlled.

Reopening existing catalogs still reads format marker 1 only; no full structural
schema validation was introduced. Reopening an index still decodes and validates
its archived descriptor. Shared CHECK creation does not claim structural CHECK
validation. Server-side PostgreSQL SQLx migration history is outside this SQLite
module's scope and is unchanged; it is not an outstanding SQLite adoption gap.
The service's applicable 07e creation scope is complete.

The workspace and core aliases now pin published
`lelloman-simple-server = "=0.1.2"` (exact compatible versions must agree);
`sct-archive` enables `database-sqlite-schema`. Core/server API/features otherwise
remain unchanged. Reviewed source:
`358a226a29d66e6f8a73df4573799c8b0970c8cd`; registry archive SHA-256:
`d31aa705a4b51e0dfea8fdc334de20f8cf6048b049dedb970ef63227a7661d46`.
No local path patch or unrelated lockfile updates are present.

Baseline archive package tests passed: one library connection-policy test,
two catalog, six reader and three restore tests (12 total). Three added tests
use frozen previous bootstrap SQL against actual SQLite to compare all table
columns/types/nullability/keys, foreign-key targets/actions and index metadata,
including ordered terms/collations. They check format CHECK/NOT NULL enforcement,
unique-key/FK rejection, nullable TEXT primary keys, existing-table failure,
rollback of all catalog phases/marker with an unchanged user_version, and actual
legacy-file reopen preserving rows and its marker-only policy. Existing reader
and restore tests cover graph-validation failure, corruption/missing media,
reuse, symlink protection, bounded input, resume and restoration.

Final checks passed: `cargo test --locked -p sct-archive` (15 tests, no
failures/ignored tests), `cargo clippy --locked -p sct-archive --all-targets -- -D warnings`,
`cargo check --locked --workspace`, `cargo fmt --all --check` and
`git diff --check`. Full PostgreSQL-backed core/server integration, web/browser,
client qualification and deployment suites were not repeated for this archive
creation change.

Implementation and documentation used an isolated worktree. `master` was rebased
onto its dedicated branch and ancestry/identical tested tree verified before
removing the owned worktree/branch. Original untracked `.validation-work/` and
`docs/step-04c-cors.md` were preserved. Nothing was pushed or deployed; tests used
isolated files/media fixtures rather than production archives.
