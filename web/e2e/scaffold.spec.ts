import { expect, test } from "@playwright/test";
import type { ErrorResponse } from "../src/generated/api";

test("built Vue application runs from the Rust server", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await expect(page).toHaveTitle("ScT — Stipa ca Truova");
  await expect(
    page.getByRole("heading", { name: "Stipa ca Truova" }),
  ).toBeVisible();
  await expect(page.getByText("In development", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("liveness is available, foundation endpoints require authentication, and uploads remain unavailable", async ({
  request,
}) => {
  const live = await request.get("/health/live");
  expect(live.status()).toBe(200);
  expect(await live.text()).toBe("ok\n");
  for (const path of [
    "/api/v1/roots",
    "/api/v1/uploads",
    "/api/v1/capabilities",
  ]) {
    const response = await request.get(path);
    const pending = path === "/api/v1/uploads";
    expect(response.status()).toBe(pending ? 404 : 401);
    expect(response.headers()["content-type"] ?? "").not.toContain("text/html");
    const error: ErrorResponse = await response.json();
    expect(error.code).toBe(pending ? "unsupported_operation" : "unauthorized");
    expect(error.request_id).toBe(response.headers()["x-request-id"]);
    expect(error.retry.kind).toBe("never");
  }
});
