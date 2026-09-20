import { expect, type Page, type APIRequestContext } from "@playwright/test";
import { createRoot, uploadFile } from "./program";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

export async function operationsWorkflow(
  page: Page,
  request: APIRequestContext,
) {
  const root = await createRoot(request, "Operations browser root");
  await uploadFile(request, root, "report.txt", "checkpoint backup bytes");
  await page.goto("/");
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page
    .getByLabel("Credential", { exact: true })
    .fill(process.env.SCT_E2E_ADMIN_TOKEN!);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.locator(`a[href="#/roots/${root.id}"]`).click();
  await page.getByRole("button", { name: "Activity", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Storage usage" }),
  ).toContainText("23 B");
  await page
    .getByRole("button", { name: "Transfer sessions", exact: true })
    .click();
  await expect(page.getByRole("table")).toContainText("committed");
  await page.getByRole("button", { name: "Integrity", exact: true }).click();
  await page.getByLabel("I acknowledge this maintenance operation").check();
  await page
    .getByRole("button", { name: "Start maintenance", exact: true })
    .click();
  await expect(page.getByRole("status")).toHaveText("Maintenance job queued.");
  await expect
    .poll(
      async () => {
        await page.getByRole("button", { name: "Refresh maintenance" }).click();
        await expect(
          page.getByRole("button", { name: "Refresh maintenance" }),
        ).toBeEnabled();
        return page.getByRole("table").textContent();
      },
      { timeout: 30000 },
    )
    .toContain("completed");
  await page
    .getByRole("button", { name: "Inspect findings", exact: true })
    .click();
  await expect(
    page.getByText("No integrity findings for this root.", { exact: true }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Operations", exact: true }).click();
  await page.getByRole("button", { name: "Checkpoints", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Create checkpoint", exact: true }),
  ).toBeDisabled();
  await page
    .getByLabel("I acknowledge the write pause and retained storage pins")
    .check();
  await page
    .getByRole("button", { name: "Create checkpoint", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Checkpoint · exported", exact: true }),
  ).toBeVisible({ timeout: 30000 });
  await expect(
    page.getByRole("heading", { name: "Catalog-only export", exact: true }),
  ).toBeVisible();
  const artifacts: Record<string, Buffer> = {};
  for (const name of ["catalog.jsonl", "inventory.jsonl", "descriptor.json"]) {
    const promise = page.waitForEvent("download");
    await page
      .getByRole("link", { name: `Download ${name}`, exact: true })
      .click();
    const download = await promise;
    expect(await download.failure()).toBeNull();
    artifacts[name] = await readFile((await download.path())!);
  }
  const descriptor = JSON.parse(artifacts["descriptor.json"].toString());
  expect(descriptor.package_state).toBe("catalog_only");
  expect(
    createHash("sha256").update(artifacts["catalog.jsonl"]).digest("hex"),
  ).toBe(descriptor.catalog_digest);
  expect(
    createHash("sha256").update(artifacts["inventory.jsonl"]).digest("hex"),
  ).toBe(descriptor.inventory_digest);
  await page
    .getByRole("button", { name: "Inspect pinned inventory", exact: true })
    .click();
  const selected = page.getByRole("region", { name: "Selected checkpoint" });
  await expect(selected.getByRole("table")).toContainText(root.id);
  await expect(
    page.getByRole("button", { name: "Release checkpoint", exact: true }),
  ).toBeDisabled();
  await page
    .getByLabel(
      "I acknowledge that releasing pins can make this checkpoint unrecoverable",
    )
    .check();
  await page
    .getByRole("button", { name: "Release checkpoint", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Checkpoint · released", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: /upload|delete files|purge|restore backup|backup now/i,
    }),
  ).toHaveCount(0);
}
