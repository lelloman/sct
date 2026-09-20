import { expect, test, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createRoot, program, uploadFile, uploadTree } from "./program";
import type { EntryInspection } from "../src/generated/api";
async function download(page: Page, link: string) {
  const pending = page.waitForEvent("download");
  await page.getByRole("link", { name: link, exact: true }).click();
  const result = await pending;
  expect(await result.failure()).toBeNull();
  return readFile((await result.path())!, "utf8");
}
test("browser reads current files, historical versions and atomic tree snapshots", async ({
  page,
  request,
}) => {
  const root = await createRoot(request);
  const old = await uploadFile(request, root, "report.txt", "old backup bytes");
  const entry = await program<EntryInspection>(
    request,
    "GET",
    `/api/v1/roots/${root.id}/resolve?path=%2Freport.txt`,
  );
  await uploadFile(request, root, "report.txt", "current backup bytes", entry);
  await uploadTree(request, root);
  await page.goto("/");
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page
    .getByLabel("Credential", { exact: true })
    .fill(process.env.SCT_E2E_ALICE_TOKEN!);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.locator(`a[href="#/roots/${root.id}"]`).click();
  await expect(
    page.getByRole("heading", { name: "Browser snapshots", exact: true }),
  ).toBeVisible();
  expect(await download(page, "Download report.txt")).toBe(
    "current backup bytes",
  );
  await page
    .getByRole("button", { name: "History of report.txt", exact: true })
    .click();
  expect(await download(page, `Download version ${old.commit_sequence}`)).toBe(
    "old backup bytes",
  );
  await page.getByRole("button", { name: "tree", exact: true }).click();
  expect(await download(page, "Download nested.txt")).toBe(
    "saved inside a tree",
  );
  await expect(
    page.getByRole("button", {
      name: /upload|rename|delete|purge|restore|backup now/i,
    }),
  ).toHaveCount(0);
  await page.screenshot({
    path: "/tmp/sct-snapshot-browser.png",
    fullPage: true,
  });
});
