import { expect, test } from "@playwright/test";

test("browser sessions survive reload, enforce roles and leave no saved program token", async ({
  page,
  context,
  request,
}) => {
  const token = process.env.SCT_E2E_ALICE_TOKEN!;
  const rejected = await request.post("/auth/session", {
    headers: {
      Authorization: `Bearer ${token}`,
      Origin: "https://foreign.invalid",
    },
  });
  expect(rejected.status()).toBe(403);
  await page.goto("/");
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page.getByLabel("Credential", { exact: true }).fill(token);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Storage roots", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Storage administration", exact: true }),
  ).toHaveCount(0);
  const cookies = await context.cookies();
  expect(
    cookies.find((cookie) => cookie.name === "sct_session")?.httpOnly,
  ).toBe(true);
  expect(cookies.find((cookie) => cookie.name === "sct_csrf")?.httpOnly).toBe(
    false,
  );
  expect(
    await page.evaluate(() => ({
      local: localStorage.length,
      session: sessionStorage.length,
    })),
  ).toEqual({ local: 0, session: 0 });
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Sign out", exact: true }),
  ).toBeVisible();
  await page.goto("/#/storage");
  await expect(
    page.getByRole("heading", { name: "Page unavailable" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Stipa ca Truova" }),
  ).toBeVisible();
  expect(
    (
      await request.get("/api/v1/account", {
        headers: {
          Cookie: `sct_session=${cookies.find((cookie) => cookie.name === "sct_session")!.value}`,
        },
      })
    ).status(),
  ).toBe(401);
});

test("administrator shell exposes management navigation without snapshot mutation controls", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByText("Sign in with a program or recovery credential", { exact: true })
    .click();
  await page
    .getByLabel("Credential", { exact: true })
    .fill(process.env.SCT_E2E_ADMIN_TOKEN!);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("link", { name: "Storage administration", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Operations", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Storage roots", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: /upload|delete files|purge|restore backup|backup now/i,
    }),
  ).toHaveCount(0);
  await page.screenshot({ path: "/tmp/sct-browser-shell.png", fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("link", { name: "My account", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});
