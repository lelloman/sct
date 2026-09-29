# Step 07b: PostgreSQL catalog migration preflight

The production `sct-core::catalog::Catalog::migrate` path now compares its
embedded SQLx migration manifest with PostgreSQL's `_sqlx_migrations` ledger
through simple-server's optional preflight before running SQLx. It verifies
version order, names, historical checksums and dirty rows. SQLx retains the
authoritative migration runner and transaction behavior. The offline archive's
fixed SQLite format marker belongs to a different protocol and has no
ordered migration ledger for 07b.

The reviewed shared revision is
`c1ff3d19d685cd267f5bc712b306f44ef4678d07`, pinned in
`simple-server.rev`. Nine core library tests passed with one existing ignore;
the non-PostgreSQL test targets passed with their database tests ignored by
default. Against an isolated PostgreSQL container, the new checksum-drift
test and the existing migrations/constraints/rollback test both passed.
Workspace formatting and strict all-target core Clippy passed. The test
container was removed. The temporary migration worktree and branch were
removed after `master` was rebased onto commit `b16ffe3`; unrelated untracked
files in the original checkout were preserved.
