import { expect, test } from "@playwright/test";
import { randomUUID } from "node:crypto";

test("administrator creates an account; owner issues and revokes a credential", async ({
  page,
  request,
}) => {
  const principal = `browser-${randomUUID()}`;
  await page.goto("/");
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page
    .getByLabel("Credential", { exact: true })
    .fill(process.env.SCT_E2E_ADMIN_TOKEN!);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("link", { name: "Accounts", exact: true }).click();
  await page.getByLabel("New account ID").fill(principal);
  await page
    .getByRole("button", { name: "Create account", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: `Manage ${principal}`, exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Issue program credential", exact: true })
    .click();
  const secret = await page.getByLabel("New credential secret").inputValue();
  expect(secret.length).toBeGreaterThan(20);
  await page
    .getByRole("button", { name: "I have saved the credential" })
    .click();
  await expect(page.getByLabel("New credential secret")).toHaveCount(0);
  await page.getByLabel("Disable account", { exact: true }).check();
  await page.getByLabel("I acknowledge this account access change").check();
  await page.getByRole("button", { name: "Save account access" }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Account updated." }),
  ).toBeVisible();
  expect(
    (
      await request.get("/api/v1/account", {
        headers: { Authorization: `Bearer ${secret}` },
      })
    ).status(),
  ).toBe(401);
  await page.getByLabel("Disable account", { exact: true }).uncheck();
  await page.getByLabel("I acknowledge this account access change").check();
  await page.getByRole("button", { name: "Save account access" }).click();
  await expect(
    page.getByLabel("I acknowledge this account access change"),
  ).not.toBeChecked();
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page.getByLabel("Credential", { exact: true }).fill(secret);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("link", { name: "My account", exact: true }).click();
  await expect(
    page.getByRole("link", { name: "Accounts", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Issue program credential", exact: true })
    .click();
  const second = await page.getByLabel("New credential secret").inputValue();
  const issued = page
    .getByRole("status")
    .filter({ hasText: "Save this credential now" });
  const id = (await issued.locator(".mono").textContent())!
    .replace("Credential ID: ", "")
    .trim();
  const auth = { Authorization: `Bearer ${second}` };
  expect(
    (await request.get("/api/v1/account", { headers: auth })).status(),
  ).toBe(200);
  await page
    .getByRole("button", { name: "I have saved the credential" })
    .click();
  await page.getByRole("button", { name: `Revoke ${id}`, exact: true }).click();
  await page
    .getByRole("button", { name: "Confirm revocation", exact: true })
    .click();
  await expect(page.getByRole("row").filter({ hasText: id })).toContainText(
    "Revoked",
  );
  expect(
    (await request.get("/api/v1/account", { headers: auth })).status(),
  ).toBe(401);
  await page.reload();
  await expect(page.getByLabel("New credential secret")).toHaveCount(0);
});
