import { expect, test } from "@playwright/test";
import { mkdtemp, rm } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { program } from "./program";
import type {
  RootPage,
  RootManagement,
  AllocationPage,
} from "../src/generated/api";

test("administrator provisions physical storage and root limits with guarded allocation resizing", async ({
  page,
  request,
}) => {
  const directory = await mkdtemp("/tmp/sct-browser-storage-");
  const suffix = randomUUID();
  const pool = `pool-${suffix}`;
  const backend = `local-${suffix}`;
  const display = `Admin root ${suffix}`;
  try {
    await page.goto("/");
    await page
      .getByText("Sign in with a program or recovery credential", {
        exact: true,
      })
      .click();
    await page
      .getByLabel("Credential", { exact: true })
      .fill(process.env.SCT_E2E_ADMIN_TOKEN!);
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await page
      .getByRole("link", { name: "Storage administration", exact: true })
      .click();
    await page.getByRole("button", { name: "Pools", exact: true }).click();
    await page.getByLabel("Pool ID", { exact: true }).fill(pool);
    await page
      .getByLabel("Pool capacity in bytes", { exact: true })
      .fill("1000000");
    await page.getByLabel("Initial staging allocation in bytes").fill("100000");
    await page.getByRole("button", { name: "Save pool", exact: true }).click();
    await expect(page.getByRole("status")).toHaveText("Pool saved.");
    await page.getByRole("button", { name: "Backends", exact: true }).click();
    await page.getByLabel("Backend ID", { exact: true }).fill(backend);
    await page
      .getByLabel("Backend display name")
      .fill("Browser provisioned local storage");
    await page.getByLabel("Backend pool ID").fill(pool);
    await page.getByLabel("Local storage directory").fill(directory);
    await page
      .getByRole("button", { name: "Save backend", exact: true })
      .click();
    await expect(page.getByRole("status")).toHaveText("Backend saved.");
    await expect(page.getByLabel("Local storage directory")).toBeDisabled();
    await page.getByRole("button", { name: "Roots", exact: true }).click();
    await page.getByLabel("Root display name").fill(display);
    await page.getByLabel("Root quota in bytes", { exact: true }).fill("10000");
    await page.getByLabel("Owner account ID").fill("alice");
    await page.getByLabel("Account mount path").fill(`/${suffix}`);
    await page.getByLabel("First backend ID").fill(backend);
    await page.getByLabel("First pool ID").fill(pool);
    await page
      .getByLabel("First pool retained allocation in bytes")
      .fill("50000");
    await page.getByRole("button", { name: "Save root", exact: true }).click();
    await expect(page.getByRole("status")).toHaveText("Root saved.");
    await page.getByLabel("Enable root", { exact: true }).check();
    await page.getByRole("button", { name: "Save root", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Save root", exact: true }),
    ).toBeEnabled();
    await page.getByLabel("Maximum delegated quota in bytes").fill("15000");
    await page.getByRole("button", { name: "Save quota ceiling" }).click();
    await expect(page.getByRole("status")).toHaveText("Quota ceiling saved.");
    await page
      .getByLabel("Maximum retained versions", { exact: true })
      .fill("5");
    await page
      .getByRole("button", { name: "Save retention constraints" })
      .click();
    await expect(page.getByRole("status")).toHaveText(
      "Policy constraints saved.",
    );
    const roots = await program<RootPage>(
      request,
      "GET",
      "/api/v1/roots?limit=100",
    );
    const root = roots.items.find((root) => root.display_name === display)!;
    expect(root.state).toBe("enabled");
    const management = await program<RootManagement>(
      request,
      "GET",
      `/api/v1/roots/${root.id}/management`,
    );
    expect(management.quota_ceiling_bytes).toBe(15000);
    expect(management.max_versions).toBe(5);
    await page
      .getByRole("button", { name: "Allocations", exact: true })
      .click();
    const allocations = await program<AllocationPage>(
      request,
      "GET",
      "/api/v1/admin/allocations?limit=100",
    );
    const allocation = allocations.items.find(
      (a) => a.owner.kind === "root" && a.owner.root_id === root.id,
    )!;
    await page
      .getByRole("button", {
        name: `Edit allocation ${allocation.id}`,
        exact: true,
      })
      .click();
    await page.getByLabel("Allocation capacity in bytes").fill("60000");
    await page.getByRole("button", { name: "Save allocation" }).click();
    await expect(page.getByRole("status")).toHaveText("Allocation saved.");
    await page.getByRole("button", { name: "Pools", exact: true }).click();
    await page
      .getByRole("button", { name: `Edit pool ${pool}`, exact: true })
      .click();
    await page.getByLabel("Pool capacity in bytes", { exact: true }).fill("1");
    await page.getByRole("button", { name: "Save pool", exact: true }).click();
    await expect(page.getByRole("alert")).toBeVisible();
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
