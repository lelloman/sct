# Step 07a SQLite connection policy

Both offline archive SQLite files (the work index and catalog) use one
`simple-server` connection policy for foreign keys, DELETE journal mode, and
FULL synchronous mode. Each new connection applies and verifies those values
before archive SQL runs. The existing private-file checks, lock, statement
cache, `cache_size=-8192`, and `temp_store=FILE` remain archive-owned. The
PostgreSQL server catalog is outside this policy.

The reviewed shared revision is in `simple-server.rev`. The archive suite
passed: one focused connection test, two catalog tests, six reader tests, and
three restore tests. Strict all-target Clippy and formatting pass. Tests used
isolated temporary files and bundled fixtures.
