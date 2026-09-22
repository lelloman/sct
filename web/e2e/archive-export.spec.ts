import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRoot, uploadFile } from "./program";

test("browser previews, exports a real ISO and records bound read-back without source mutation", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  const scratch = mkdtempSync(join(tmpdir(), "sct-browser-iso-"));
  try {
    const root = await createRoot(request, "Optical export source");
    await uploadFile(
      request,
      root,
      "backup.txt",
      "program-created backup bytes",
    );
    await page.goto("/");
    await page
      .getByText("Sign in with a program or recovery credential", {
        exact: true,
      })
      .click();
    await page
      .getByLabel("Credential", { exact: true })
      .fill(process.env.SCT_E2E_ALICE_TOKEN!);
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await page
      .getByRole("link", { name: "Disc archives", exact: true })
      .click();
    await page.getByText("Create a collection", { exact: true }).click();
    await page
      .getByLabel("Collection label", { exact: true })
      .fill("Export workflow");
    await page
      .getByRole("button", { name: "Create collection", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Export workflow", exact: true }),
    ).toBeVisible();
    const collection = new URL(page.url()).hash.split("/").at(-1)!;
    const mutations: string[] = [];
    page.on("request", (req) => {
      if (
        /\/api\/v1\/roots\//.test(req.url()) &&
        !["GET", "HEAD"].includes(req.method())
      )
        mutations.push(req.url());
    });
    await page.getByText("Plan the next disc set", { exact: true }).click();
    const sourceRoot = page.getByLabel("Source root", { exact: true });
    await expect(sourceRoot).toBeEnabled({ timeout: 10_000 });
    await sourceRoot.selectOption({ index: 1 });
    await page.getByLabel("Source path", { exact: true }).fill("/backup.txt");
    await page
      .getByRole("button", { name: "Find saved versions", exact: true })
      .click();
    await expect(
      page.getByLabel("Saved version", { exact: true }).locator("option"),
    ).toHaveCount(2);
    await page
      .getByLabel("Saved version", { exact: true })
      .selectOption({ index: 1 });
    await page
      .getByRole("button", { name: "Add source to plan", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Create preview", exact: true })
      .click();
    const confirm = page.getByRole("button", {
      name: "Confirm and generate ISOs",
      exact: true,
    });
    await expect(confirm).toBeVisible({ timeout: 60_000 });
    await expect(
      page.getByText("Complete index repeated on every new disc", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByText("Recovery software repeated on every new disc", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Disc 1", exact: true }),
    ).toBeVisible();
    await confirm.click();
    await expect(
      page.getByText("Images are ready.", { exact: false }),
    ).toBeVisible({ timeout: 90_000 });
    await page.getByRole("button", { name: "Disc 1", exact: true }).click();
    const downloadEvent = page.waitForEvent("download");
    await page
      .getByRole("button", { name: "Download ISO", exact: true })
      .click();
    const download = await downloadEvent;
    const iso = join(scratch, "disc.iso");
    await download.saveAs(iso);
    expect(await download.failure()).toBeNull();
    const media = join(scratch, "media");
    mkdirSync(media);
    execFileSync(
      join(process.cwd(), "..", "scripts", "materialize-archive-media"),
      [iso, media],
      { stdio: "pipe" },
    );
    const tool = join(media, "recovery", "sct-recover");
    const work = join(scratch, "original.sqlite");
    const report = JSON.parse(
      execFileSync(tool, ["verify-report", media, work], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }),
    );
    expect(report.metadata_verified).toBe(true);
    expect(report.verified_objects).toBe(report.expected_objects);
    expect(report.collection_id).toBe(collection);
    // Validate recovery using only the downloaded image and the tool on it.
    const cache = join(scratch, "cache");
    const restored = join(scratch, "restored");
    mkdirSync(cache);
    mkdirSync(restored);
    execFileSync(tool, ["ingest", media, work, cache], {
      stdio: "pipe",
    });
    execFileSync(tool, ["restore", work, cache, restored], { stdio: "pipe" });
    expect(readFileSync(join(restored, "backup.txt"), "utf8")).toBe(
      "program-created backup bytes",
    );
    const registerSummary = page.getByText("Record a physical copy", {
      exact: true,
    });
    await registerSummary.click();
    const register = registerSummary.locator("..");
    await register
      .getByLabel("Copy label", { exact: true })
      .fill("Read-back fixture copy");
    await register
      .getByLabel("Physical location", { exact: true })
      .fill("Test shelf");
    await register
      .getByRole("button", { name: "Register physical copy", exact: true })
      .click();
    const manageSummary = page.getByText("Manage copy Read-back fixture copy", {
      exact: true,
    });
    await expect(manageSummary).toBeVisible();
    await manageSummary.click();
    const manage = manageSummary.locator("..");
    await manage
      .getByLabel("Verification summary JSON", { exact: true })
      .fill(JSON.stringify({ ...report, disc_id: "wrong_disc" }));
    await manage
      .getByRole("button", { name: "Record read-back report", exact: true })
      .click();
    await expect(manage.getByRole("alert")).toContainText("does not match");
    await manage
      .getByLabel("Verification summary JSON", { exact: true })
      .fill(JSON.stringify(report));
    await manage
      .getByRole("button", { name: "Record read-back report", exact: true })
      .click();
    await expect(
      page.getByText("Current health:", { exact: false }),
    ).toContainText("read back verified");
    expect(mutations).toEqual([]);
    await page.reload();
    await expect(
      page.getByText("Latest historically completed recovery point:", {
        exact: false,
      }),
    ).toBeVisible();
  } finally {
    // xorriso preserves read-only disc directory permissions.
    execFileSync("chmod", ["-R", "u+rwX", scratch]);
    rmSync(scratch, { recursive: true, force: true });
  }
});
