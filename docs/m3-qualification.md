# M3 qualification

M3 tasks SCT-28–41 implement atomic directory snapshots, durable complete-replacement
and patch drafts, shared-graph admission/reclamation, historical traversal and
pinned reads, hide, separately authorized purge, policy preview/apply and
acknowledged independent-policy moves. HTTP and standalone Rust SDK are delivered.
Browser controls, actual checkpoint creation/restore, populated mode conversion
and M5 release qualification remain outside this milestone.

## Reproducible checks

- `./scripts/check`: M1/M2/M3 Rust and HTTP tests, strict Clippy, generated
  contracts, independent SDK packaging, real PostgreSQL/S3 and browser acceptance.
- `./scripts/test-trees`: SDK-only replacement/patch/history/policy/hide/purge on
  local, S3, local+local, local+S3 and S3+S3 placements. Includes the M2 real-I/O
  lost-response, corruption/fallback, pinned-reader and capacity regression suite.
- `./scripts/qualify-tree-scale`: 10,000 and 1,000,000-entry independent subtrees,
  fully previewed and applied across a namespace boundary. The preview restarts
  after 25 bounded batches; persisted cursor equality is asserted before resuming.
  RSS is read inside the Rust worker; PostgreSQL is a separate disposable process.

Core PostgreSQL cases cover graph sharing, snapshot rotation/publication races,
whole-tree pre-payload admission, draft restart/identity validation, hidden retention,
stale/expired previews, nested policy overrides, cross-policy moves, separate purge
authorization, pin/retirement ordering, unknown-delete charging and authorization
revocation/resume. The SDK HTTP scenario reopens an interrupted mutation plan,
rejects changed source bytes, replays commit receipts, paginates immutable history,
downloads a range and applies explicit lifecycle requests.

## Scale fixture interpretation

The million-entry fixture consists of empty independent directories under one
source directory. It loads metadata directly with user triggers temporarily
disabled inside a fixture transaction, then advances the root mutation generation
once. Production preview/apply code performs the entire traversal and namespace
publication. Every preview pass reads at most 128 children, and the wide-tree
frontier never exceeds one row. Measurements cover this metadata lifecycle path;
they do not claim a million-file payload upload or checkpoint restore benchmark.
Tree manifests separately use 128-entry leaves and 64-reference branches, while
graph, version-retirement and purge frontiers have fixed batch limits.

At M3 handoff, M4 and M5 were pending. M4 is now qualified separately in
[M4 qualification](m4-qualification.md); M5 remains explicitly pending.

## Recorded results (2026-09-19)

Linux x86-64, Rust 1.96, PostgreSQL 17, debug test build:

| Entries | Preview batches | Fixture load | Preview including restart | Baseline RSS | Peak sampled RSS |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | 80 | 0.623 s | 1.043 s | 14,608 KiB | 14,608 KiB |
| 1,000,000 | 7,814 | 53.864 s | 84.808 s | 14,792 KiB | 14,936 KiB |

Both runs resumed after releasing/reacquiring the writer at batch 25, completed
the full preview and successfully applied the move. RSS samples are taken every
100 batches (the short run records its baseline); these are process measurements,
not a formal worst-case bound or PostgreSQL memory measurements. The million-entry
run added 144 KiB over its baseline while traversing 100 times more entries.

Real-placement qualification passed for all five placements. Each runs complete
tree replacement, interrupted durable-plan replay/source-drift rejection, patch,
historical browse/range read, acknowledged policy retirement, hide and purge.

Final `./scripts/check` exited 0: formatting, strict workspace Clippy, Rust tests,
generated contract drift/fixtures, frontend checks/build, standalone SDK package,
27 PostgreSQL foundation cases plus lease/HTTP/process tests, real MinIO conformance,
and 5 passing browser/API acceptance cases. M4/M5 are the two explicitly skipped
gates. The M3 real-placement run additionally lowers quota, attempts a tree upload
and verifies zero physical copies were created before capacity rejection.
