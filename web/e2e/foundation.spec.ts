import { expect, test, type Page } from "@playwright/test";
import type {
  RootInspection,
  EntryInspection,
  EntryPage,
  EffectivePolicy,
  GrantInspection,
} from "../src/generated/api";

const tokens = {
  admin: process.env.SCT_E2E_ADMIN_TOKEN!,
  alice: process.env.SCT_E2E_ALICE_TOKEN!,
  bob: process.env.SCT_E2E_BOB_TOKEN!,
};
async function api<T>(
  page: Page,
  role: keyof typeof tokens,
  method: string,
  path: string,
  body?: unknown,
): Promise<{ status: number; body: T }> {
  return page.evaluate(
    async ({ token, method, path, body }) => {
      const response = await fetch(path, {
        method,
        headers: {
          Authorization: `Bearer ${token}`,
          "Content-Type": "application/json",
        },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      return {
        status: response.status,
        body: response.status === 204 ? null : await response.json(),
      };
    },
    { token: tokens[role], method, path, body },
  );
}

test("M1: provision roots and enforce grants, namespace and policy boundaries", async ({
  page,
}) => {
  expect(tokens.admin).toBeTruthy();
  await page.goto("/");
  const rootRequest = {
    idempotency_key: "browser_root",
    owner_id: "alice",
    display_name: "Alice storage",
    mount_path: "/foundation",
    placement: {
      kind: "single",
      backend_id: "test_backend",
      read_policy: {
        candidates: [
          {
            backend_id: "test_backend",
            connection_timeout_ms: 100,
            initial_response_timeout_ms: 100,
            idle_timeout_ms: 1000,
            throughput_window_ms: 1000,
            minimum_bytes_per_second: 1,
            startup_allowance_ms: 1000,
          },
        ],
        overall_deadline_ms: null,
      },
    },
    quota_bytes: 5000,
    default_policy: { mode: "strict", keep_versions: 1 },
    allocations: [{ pool_id: "pool", retained_capacity_bytes: 5000 }],
  };
  const created = await api<RootInspection>(
    page,
    "admin",
    "POST",
    "/api/v1/roots",
    rootRequest,
  );
  expect(created.status).toBe(200);
  const root = created.body;
  expect(root.state).toBe("disabled");
  const enabled = await api<RootInspection>(
    page,
    "admin",
    "PATCH",
    `/api/v1/roots/${root.id}`,
    {
      expected_revision: root.revision,
      display_name: root.display_name,
      state: "enabled",
      placement: root.placement,
      quota_bytes: root.quota_bytes,
    },
  );
  expect(enabled.body.state).toBe("enabled");
  expect(
    (await api(page, "bob", "GET", `/api/v1/roots/${root.id}`)).status,
  ).toBe(404);
  const grant = await api<GrantInspection>(
    page,
    "alice",
    "POST",
    "/api/v1/admin/grants",
    { root_id: root.id, principal_id: "bob", capabilities: ["list_read"] },
  );
  expect(grant.status).toBe(200);
  const resolve = () =>
    api<EntryInspection>(
      page,
      "alice",
      "GET",
      `/api/v1/roots/${root.id}/resolve?path=%2F`,
    );
  let parent = (await resolve()).body;
  const directoryRequest = {
    idempotency_key: "browser_directory",
    parent_id: parent.id,
    expected_parent_revision: parent.revision,
    name: "Documents",
    versioning: "independent_files",
  };
  expect(
    (
      await api(
        page,
        "bob",
        "POST",
        `/api/v1/roots/${root.id}/entries`,
        directoryRequest,
      )
    ).status,
  ).toBe(403);
  const directory = await api<EntryInspection>(
    page,
    "alice",
    "POST",
    `/api/v1/roots/${root.id}/entries`,
    directoryRequest,
  );
  expect(directory.status).toBe(200);
  parent = (await resolve()).body;
  const renamed = await api<EntryInspection>(
    page,
    "alice",
    "POST",
    `/api/v1/entries/${directory.body.id}/rename`,
    {
      expected_revision: directory.body.revision,
      expected_source_parent_revision: parent.revision,
      destination_parent_id: parent.id,
      expected_destination_parent_revision: parent.revision,
      name: "Archive",
    },
  );
  expect(renamed.status).toBe(200);
  expect(renamed.body.id).toBe(directory.body.id);
  const listing = await api<EntryPage>(
    page,
    "bob",
    "GET",
    `/api/v1/roots/${root.id}/entries?directory_id=${parent.id}&limit=10`,
  );
  expect(listing.body.items[0].name).toBe("Archive");
  const policy = await api<EffectivePolicy>(
    page,
    "alice",
    "GET",
    `/api/v1/policies?kind=entry&root_id=${root.id}&entry_id=${renamed.body.id}`,
  );
  const request = {
    scope: { kind: "entry", root_id: root.id, entry_id: renamed.body.id },
    expected_revision: policy.body.revision,
    expected_scope_revision: renamed.body.revision,
    policy: { mode: "strict", keep_versions: 3 },
    versioning: "independent_files",
  };
  expect(
    (await api(page, "bob", "PATCH", "/api/v1/policies", request)).status,
  ).toBe(403);
  const updated = await api<EffectivePolicy>(
    page,
    "alice",
    "PATCH",
    "/api/v1/policies",
    request,
  );
  expect(updated.body.policy.keep_versions).toBe(3);
  expect(
    (await api(page, "alice", "PATCH", "/api/v1/policies", request)).status,
  ).toBe(409);
  expect(
    (
      await api(
        page,
        "alice",
        "DELETE",
        `/api/v1/admin/grants/${grant.body.id}?expected_revision=${grant.body.revision}`,
      )
    ).status,
  ).toBe(204);
  expect(
    (await api(page, "bob", "GET", `/api/v1/entries/${renamed.body.id}`))
      .status,
  ).toBe(404);
  expect(
    (
      await api(
        page,
        "alice",
        "GET",
        `/api/v1/roots/${root.id}/resolve?path=%2F..%2Fsecret`,
      )
    ).status,
  ).toBe(400);
});
