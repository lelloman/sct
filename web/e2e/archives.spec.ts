import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { createRoot } from "./program";
async function signIn(page: Page, token: string) {
  await page.goto("/");
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page.getByLabel("Credential", { exact: true }).fill(token);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Storage roots", exact: true }),
  ).toBeVisible();
}
test("archive history keeps older disc dependencies, copy health, pagination and account boundaries", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const root = await createRoot(request, "Archive source");
  await signIn(page, process.env.SCT_E2E_ALICE_TOKEN!);
  await page.getByRole("link", { name: "Disc archives", exact: true }).click();
  await page.getByText("Create a collection", { exact: true }).click();
  await page
    .getByLabel("Collection label", { exact: true })
    .fill("Permanent optical history");
  await page
    .getByRole("button", { name: "Create collection", exact: true })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Permanent optical history",
      exact: true,
    }),
  ).toBeVisible();
  const collection = new URL(page.url()).hash.split("/").at(-1)!;
  execFileSync(
    "cargo",
    [
      "run",
      "--locked",
      "--quiet",
      "-p",
      "sct-server",
      "--example",
      "archive-browser-fixture",
      "--",
      collection,
      root.id,
    ],
    { cwd: "..", env: process.env, timeout: 120_000 },
  );
  await page.reload();
  await expect(
    page.getByText("Next disc:", { exact: false }).first(),
  ).toContainText("23");
  const history = page.getByRole("table").first();
  await expect(
    history.getByRole("button", { name: "Snapshot 2", exact: true }),
  ).toBeVisible();
  await history
    .getByRole("button", { name: "Snapshot 2", exact: true })
    .click();
  const snapshot = page.getByRole("region", { name: "Snapshot details" });
  await snapshot.getByRole("button", { name: "Disc 2", exact: true }).click();
  const disc = page.getByRole("region", { name: "Disc details" });
  await expect(
    disc.getByText("Current health:", { exact: false }),
  ).toContainText("missing");
  await expect(disc.getByText("Cached ISO:", { exact: false })).toContainText(
    "expired",
  );
  await expect(
    disc.getByRole("button", { name: "Snapshot 2", exact: true }),
  ).toBeVisible();
  await expect(
    disc.getByRole("button", { name: "Snapshot 22", exact: true }),
  ).toHaveCount(0);
  await snapshot
    .getByText("Archived file and directory index", { exact: true })
    .click();
  await expect(
    snapshot.getByRole("cell", { name: "/historical-2.txt", exact: true }),
  ).toBeVisible();
  // This version was never inserted into the online catalog: historical metadata
  // remains accessible independently of current online version retention.
  await page
    .getByRole("button", { name: "Next page", exact: true })
    .first()
    .click();
  await expect(
    history.getByRole("button", { name: "Snapshot 22", exact: true }),
  ).toBeVisible();
  await history
    .getByRole("button", { name: "Snapshot 22", exact: true })
    .click();
  await expect(
    snapshot.getByRole("button", { name: "Disc 1", exact: true }),
  ).toBeVisible();
  await expect(
    snapshot.getByRole("button", { name: "Disc 2", exact: true }),
  ).toHaveCount(0);
  await snapshot.getByLabel("Find a disc on this page").fill("22");
  await expect(
    snapshot.getByRole("button", { name: "Disc 1", exact: true }),
  ).toHaveCount(0);
  await snapshot.getByLabel("Find a disc on this page").fill("");
  await snapshot.getByRole("button", { name: "Disc 1", exact: true }).click();
  await expect(
    disc.getByRole("button", { name: "Snapshot 1", exact: true }),
  ).toBeVisible();
  await expect(
    disc.getByRole("button", { name: "Snapshot 22", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /retire|delete snapshot|upload files/i }),
  ).toHaveCount(0);
  const denied = await request.get(`/api/v1/archives/${collection}`, {
    headers: { Authorization: `Bearer ${process.env.SCT_E2E_BOB_TOKEN!}` },
  });
  expect([403, 404]).toContain(denied.status());
  const deniedDisc = await request.get(
    `/api/v1/archives/${collection}/discs/${collection}_disc_01/sets?limit=1`,
    { headers: { Authorization: `Bearer ${process.env.SCT_E2E_BOB_TOKEN!}` } },
  );
  expect([403, 404]).toContain(deniedDisc.status());
  const first = await request.get(
    `/api/v1/archives/${collection}/discs/${collection}_disc_01/sets?limit=1`,
    {
      headers: { Authorization: `Bearer ${process.env.SCT_E2E_ALICE_TOKEN!}` },
    },
  );
  expect(first.ok()).toBeTruthy();
  const firstPage = await first.json();
  expect(firstPage.sets).toHaveLength(1);
  expect(firstPage.next_cursor).toBeTruthy();
  const second = await request.get(
    `/api/v1/archives/${collection}/discs/${collection}_disc_01/sets?limit=1&cursor=${encodeURIComponent(firstPage.next_cursor)}`,
    {
      headers: { Authorization: `Bearer ${process.env.SCT_E2E_ALICE_TOKEN!}` },
    },
  );
  expect((await second.json()).sets[0].id).not.toBe(firstPage.sets[0].id);
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await signIn(page, process.env.SCT_E2E_BOB_TOKEN!);
  await page.goto(`/#/archives/${collection}`);
  await expect(page.getByRole("alert").first()).toBeVisible();
  await expect(
    page.getByRole("heading", {
      name: "Permanent optical history",
      exact: true,
    }),
  ).toHaveCount(0);
});
