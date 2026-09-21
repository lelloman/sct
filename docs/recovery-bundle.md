# Building and qualifying recovery bundles

The first supported recovery executable is Linux x86_64 with glibc 2.36 or newer,
qualified on Debian 12. It includes SQLite and needs no database service, compiler,
network connection or package installation. It is not a bootable operating system;
macOS, Windows and other CPU architectures are not currently qualified. Copy the
executable off media mounted `noexec` and make the local copy executable.

Build prerequisites are Docker, Rust 1.96, Python 3, the locked dependencies in the
local Cargo cache, and locally available `rust:1.96-bookworm` and
`debian:bookworm-slim` images. Dependency acquisition is a build preparation step,
never a recovery step. The builder resolves the build image to its immutable ID,
vendors the independent protocol/recovery workspace, and compiles in a container
with networking disabled. The standalone source includes Cargo.lock, vendor
configuration, dependency sources and their license files. It has no dependency on
the ScT server, core, adjacent simple-server repository or PostgreSQL.

```sh
./scripts/build-recovery-bundle /absolute/new/bundle v1-release1
./scripts/test-recovery-bundle /absolute/new/bundle
```

The output directory must not exist. Treat a nonzero build exit as unusable output.
The bundle manifest is published last; all artifact lengths and SHA-256 values are
fixed there. The builder prints the manifest SHA-256 and total artifact byte count.
Planning must reserve these exact bytes plus metadata/filesystem overhead and freeze
the manifest identity before publishing an ISO. Never update an existing disc or
frozen plan to point at a newer recovery tool. Source archive SHA-256, executable
hash, build image ID and base repository revision provide artifact provenance;
the source archive identifies the exact source even when the worktree is dirty.
The source archive has normalized timestamps/owners; compiler byte reproducibility
across different build environments is not asserted.

The qualification script installs the exact bundle on every canonical fixture disc,
updates only their descriptors' bundle identities, and runs the packaged executable
as a non-root user in a read-only container with networking disabled and capabilities
dropped. It exercises inspection, verification, missing-media refusal, partial
restoration, complete restoration after additional media, and later historical
points that no longer require an earlier disc. These are directory-media tests;
actual ISO filesystem and physical burn tests remain separate qualifications.

ScT currently has no declared standalone source license. The bundle states this
explicitly rather than inventing a license grant. Third-party licensing metadata
and original vendored license files accompany the executable.
