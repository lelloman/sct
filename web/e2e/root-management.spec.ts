import { expect, test } from "@playwright/test";
import { createRoot, uploadFile, program } from "./program";
import type {
  EntryInspection,
  GrantInspection,
  RootInspection,
  RootManagement,
} from "../src/generated/api";

test("owner manages quota, previews retention and delegates only allowed access", async ({
  page,
  request,
}) => {
  const root = await createRoot(request, "Managed browser root");
  await uploadFile(request, root, "report.txt", "old backup");
  const entry = await program<EntryInspection>(
    request,
    "GET",
    `/api/v1/roots/${root.id}/resolve?path=%2Freport.txt`,
  );
  await uploadFile(request, root, "report.txt", "new backup", entry);
  await page.goto("/");
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page
    .getByLabel("Credential", { exact: true })
    .fill(process.env.SCT_E2E_ALICE_TOKEN!);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.locator(`a[href="#/roots/${root.id}"]`).click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByLabel("Display name", { exact: true })
    .fill("Updated browser root");
  await page.getByLabel("Quota in bytes", { exact: true }).fill("9000");
  await page.getByLabel("Expected backup interval in seconds").fill("3600");
  await page.getByRole("button", { name: "Save root settings" }).click();
  await expect(page.getByRole("status")).toHaveText("Root settings saved.");
  const management = await program<RootManagement>(
    request,
    "GET",
    `/api/v1/roots/${root.id}/management`,
  );
  expect(management.quota_bytes).toBe(9000);
  expect(management.expected_upload_interval_seconds).toBe(3600);
  await page.getByRole("button", { name: "Retention", exact: true }).click();
  await page.getByLabel("Versions to retain").fill("1");
  await page.getByRole("button", { name: "Preview policy change" }).click();
  const preview = page.getByRole("region", { name: "Policy preview" });
  await expect(preview).toContainText("Versions to retire: 1");
  await expect(
    page.getByRole("button", { name: "Apply reviewed policy" }),
  ).toBeDisabled();
  await page
    .getByLabel("I acknowledge the retention effects shown above")
    .check();
  await page.getByRole("button", { name: "Apply reviewed policy" }).click();
  await expect(
    page.getByRole("heading", { name: "Policy job · completed" }),
  ).toBeVisible();
  expect(
    (await program<RootInspection>(request, "GET", `/api/v1/roots/${root.id}`))
      .default_policy.keep_versions,
  ).toBe(1);
  await page.getByRole("button", { name: "Grants", exact: true }).click();
  await page.getByLabel("Grant account ID").fill("bob");
  await page.getByLabel("Browse and download", { exact: true }).check();
  await page.getByRole("button", { name: "Save grant", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Grant saved.");
  const bobRoot = await program<RootInspection>(
    request,
    "GET",
    `/api/v1/roots/${root.id}`,
    undefined,
    process.env.SCT_E2E_BOB_TOKEN!,
  );
  expect(bobRoot.capabilities).toEqual(["list_read"]);
  await page
    .getByRole("button", { name: "Revoke grant for bob", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Confirm grant revocation", exact: true })
    .click();
  await expect(page.getByRole("status")).toHaveText("Grant revoked.");
  // A delegated reader cannot see management controls even on a direct root link.
  await program<GrantInspection>(request, "POST", "/api/v1/admin/grants", {
    root_id: root.id,
    principal_id: "bob",
    capabilities: ["list_read"],
  });
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page
    .getByLabel("Credential", { exact: true })
    .fill(process.env.SCT_E2E_BOB_TOKEN!);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.goto(`/#/roots/${root.id}`);
  await expect(
    page.getByRole("heading", { name: "Updated browser root", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /^(Settings|Retention|Grants)$/ }),
  ).toHaveCount(0);
});
