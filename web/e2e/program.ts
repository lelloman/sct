// Program-side fixture creation. Browser product flows never call these writers.
import type { APIRequestContext } from "@playwright/test";
import { createHash, randomUUID } from "node:crypto";
import type {
  RootInspection,
  EntryInspection,
  EffectivePolicy,
  TransferStatus,
  CommitReceipt,
  TreeDraftStatus,
  DraftIdentities,
} from "../src/generated/api";
export async function program<T>(
  request: APIRequestContext,
  method: string,
  path: string,
  data?: unknown,
  token = process.env.SCT_E2E_ADMIN_TOKEN!,
): Promise<T> {
  const response = await request.fetch(path, {
    method,
    data,
    headers: {
      Authorization: `Bearer ${token}`,
      ...(Buffer.isBuffer(data)
        ? { "Content-Type": "application/octet-stream" }
        : {}),
    },
  });
  if (!response.ok())
    throw new Error(
      `${method} ${path}: ${response.status()} ${await response.text()}`,
    );
  return (response.status() === 204 ? undefined : await response.json()) as T;
}
export async function createRoot(
  request: APIRequestContext,
  display = "Browser snapshots",
  owner = "alice",
) {
  const root = await program<RootInspection>(request, "POST", "/api/v1/roots", {
    idempotency_key: randomUUID(),
    owner_id: owner,
    display_name: display,
    mount_path: `/${randomUUID()}`,
    quota_bytes: 10000,
    default_policy: { mode: "strict", keep_versions: 2 },
    allocations: [{ pool_id: "browser_pool", retained_capacity_bytes: 100000 }],
    placement: {
      kind: "single",
      backend_id: "browser_local",
      read_policy: {
        candidates: [
          {
            backend_id: "browser_local",
            connection_timeout_ms: 5000,
            initial_response_timeout_ms: 5000,
            idle_timeout_ms: 5000,
            throughput_window_ms: 1000,
            minimum_bytes_per_second: 1,
            startup_allowance_ms: 5000,
          },
        ],
        overall_deadline_ms: null,
      },
    },
  });
  return program<RootInspection>(request, "PATCH", `/api/v1/roots/${root.id}`, {
    expected_revision: root.revision,
    display_name: root.display_name,
    state: "enabled",
    placement: root.placement,
    quota_bytes: root.quota_bytes,
  });
}
function sorted(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sorted);
  if (value && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, value]) => [key, sorted(value)]),
    );
  return value;
}
function manifest(bytes: Buffer) {
  const digest = createHash("sha256").update(bytes).digest("hex");
  const page = Buffer.from(
    JSON.stringify(
      sorted({
        format: 1,
        kind: "chunks",
        chunks: [{ digest, length: bytes.length }],
      }),
    ),
  );
  const pageId = createHash("sha256").update(page).digest("hex");
  return {
    digest,
    page,
    pageId,
    file: {
      format: 1,
      profile: "fastcdc-v2020-3.2.1-n1-s0-256k-1m-4m",
      length: bytes.length,
      chunks: 1,
      height: 0,
      tree: pageId,
    },
  };
}
async function finish(
  request: APIRequestContext,
  root: RootInspection,
  upload: string,
  bytes: Buffer,
  digest: string,
) {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    const result = await program<TransferStatus>(
      request,
      "POST",
      `/api/v1/roots/${root.id}/uploads/${upload}/admit`,
    );
    if (result.admission === "admitted") {
      ready = true;
      break;
    }
  }
  if (!ready) throw new Error("Fixture admission did not complete");
  await program(
    request,
    "PUT",
    `/api/v1/roots/${root.id}/uploads/${upload}/chunks/${digest}`,
    bytes,
  );
  ready = false;
  for (let i = 0; i < 100; i++) {
    const result = await program<TransferStatus>(
      request,
      "POST",
      `/api/v1/roots/${root.id}/uploads/${upload}/seal`,
    );
    if (result.upload.state === "ready") {
      ready = true;
      break;
    }
  }
  if (!ready) throw new Error("Fixture sealing did not complete");
  return program<CommitReceipt>(
    request,
    "POST",
    `/api/v1/roots/${root.id}/uploads/${upload}/commit`,
  );
}
export async function uploadFile(
  request: APIRequestContext,
  root: RootInspection,
  name: string,
  text: string,
  replace?: EntryInspection,
) {
  const parent = await program<EntryInspection>(
    request,
    "GET",
    `/api/v1/roots/${root.id}/resolve?path=%2F`,
  );
  const policy = await program<EffectivePolicy>(
    request,
    "GET",
    `/api/v1/policies?kind=entry&root_id=${root.id}&entry_id=${replace?.id ?? parent.id}`,
  );
  const bytes = Buffer.from(text);
  const m = manifest(bytes);
  const created = await program<TransferStatus>(
    request,
    "POST",
    `/api/v1/roots/${root.id}/uploads`,
    {
      idempotency_key: randomUUID(),
      target: replace
        ? {
            kind: "replace",
            entry_id: replace.id,
            expected_revision: replace.revision,
          }
        : {
            kind: "create",
            parent_id: parent.id,
            expected_parent_revision: parent.revision,
            name,
          },
      expected_policy_revision: policy.revision,
      manifest: m.file,
    },
  );
  await program(
    request,
    "PUT",
    `/api/v1/roots/${root.id}/uploads/${created.upload.id}/pages/${m.pageId}`,
    m.page,
  );
  return finish(request, root, created.upload.id, bytes, m.digest);
}
export async function uploadTree(
  request: APIRequestContext,
  root: RootInspection,
) {
  const parent = await program<EntryInspection>(
    request,
    "GET",
    `/api/v1/roots/${root.id}/resolve?path=%2F`,
  );
  const directory = await program<EntryInspection>(
    request,
    "POST",
    `/api/v1/roots/${root.id}/entries`,
    {
      idempotency_key: randomUUID(),
      parent_id: parent.id,
      expected_parent_revision: parent.revision,
      name: "tree",
      versioning: "atomic_directory",
    },
  );
  const policy = await program<EffectivePolicy>(
    request,
    "GET",
    `/api/v1/policies?kind=entry&root_id=${root.id}&entry_id=${directory.id}`,
  );
  const draft = await program<TreeDraftStatus>(
    request,
    "POST",
    `/api/v1/roots/${root.id}/tree-drafts`,
    {
      idempotency_key: randomUUID(),
      base: {
        root_id: root.id,
        directory_id: directory.id,
        expected_revision: directory.revision,
        base_version_id: null,
        mode: "complete_replacement",
      },
      expected_policy_revision: policy.revision,
    },
  );
  const identities = await program<DraftIdentities>(
    request,
    "POST",
    `/api/v1/roots/${root.id}/tree-drafts/${draft.id}/identities`,
    { idempotency_key: randomUUID(), count: 1 },
  );
  const bytes = Buffer.from("saved inside a tree");
  const m = manifest(bytes);
  await program(
    request,
    "PUT",
    `/api/v1/roots/${root.id}/tree-drafts/${draft.id}/pages/${m.pageId}`,
    m.page,
  );
  await program(
    request,
    "POST",
    `/api/v1/roots/${root.id}/tree-drafts/${draft.id}/mutations`,
    {
      idempotency_key: randomUUID(),
      mutations: [
        {
          kind: "upsert",
          identity: { kind: "new", entry_id: identities.entry_ids[0] },
          parent_id: directory.id,
          name: "nested.txt",
          entry_kind: "file",
          file: m.file,
        },
      ],
    },
  );
  await program(
    request,
    "POST",
    `/api/v1/roots/${root.id}/tree-drafts/${draft.id}/freeze`,
  );
  for (let i = 0; i < 100; i++) {
    const status = await program<TreeDraftStatus>(
      request,
      "POST",
      `/api/v1/roots/${root.id}/tree-drafts/${draft.id}/admit`,
    );
    if (status.admission === "admitted") break;
    if (i === 99) throw new Error("Tree fixture admission did not complete");
  }
  await program(
    request,
    "PUT",
    `/api/v1/roots/${root.id}/uploads/${draft.id}/chunks/${m.digest}`,
    bytes,
  );
  for (let i = 0; i < 100; i++) {
    const status = await program<TransferStatus>(
      request,
      "POST",
      `/api/v1/roots/${root.id}/uploads/${draft.id}/seal`,
    );
    if (status.upload.state === "ready") break;
    if (i === 99) throw new Error("Tree fixture sealing did not complete");
  }
  return program<CommitReceipt>(
    request,
    "POST",
    `/api/v1/roots/${root.id}/uploads/${draft.id}/commit`,
  );
}
