# Step 01: Axum centralization

The server and its tests use public `simple-server` Git revision
`46a36391ccca522f3ec9aa24206f2b231162dd96` and its transitional Axum re-export.
Axum remains exactly 0.8.9; the client/protocol crates retain their independent
distribution boundary.

This migration is on branch `simple-server-step01` in the isolated `sct-step01`
worktree, based on 5cffda9. The main checkout had active uncommitted S3 work in
core, server configuration and the lockfile. That work was left untouched.
This branch is not merged; reconcile any new Axum imports and lockfile changes
when integrating with ongoing development.

Validation on 2026-09-19, Rust 1.96 and Node 24.13:

`SCT_E2E_PORT=19083 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 scripts/check`
passed before and after migration. This includes:

- 33 Rust tests including Postgres integration, plus one doctest.
- Strict all-target/all-feature Clippy, formatting and workspace builds.
- Generated-contract verification and all 49 JavaScript contract tests.
- Frontend formatting/lint/type checks and production build.
- Independent protocol/client packaging and standalone-client compilation.
- Three real-server Chromium E2E scenarios for the built UI, liveness/auth,
  root provisioning, grants, namespace and policy boundaries.

Four future milestone acceptance scenarios are explicitly marked pending and
skipped by the existing suite. This migration does not implement those milestones.
The tests used temporary Postgres containers and isolated fixture data, with
cleanup handled by the scripts. The dependency graph contains only central
Axum 0.8.9. No deployment or publication was performed.

Step 02 subsequently adopts shared lifecycle helpers and a reviewed sibling
checkout; see [the current lifecycle migration](step-02-lifecycle.md). The Git
pin above describes the historical Step 01 state.
