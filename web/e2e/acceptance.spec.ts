import { expect, test } from "@playwright/test";

// Milestone gates, not passing tests. Expand these into real API/SDK/browser
// workflows using ARCHITECTURE_PLAN.md §16 as each capability lands.
for (const scenario of [
  "M3: publish atomic trees and enforce retention with shared staging limits",
  "M4: administer storage in the browser and recover an isolated checkpoint",
  "M5: qualify scale, real backend failures and independent SDK distribution",
]) {
  test.fixme(scenario, async () => {
    throw new Error(
      "Pending implementation; replace with real acceptance assertions",
    );
  });
}

// M2 is an API/SDK capability. Its real-placement harness opens actual HTTP
// connections; the browser upload control surface belongs to M4.
test("M2: resume real mirrored transfers and verify retained/downloaded bytes", async () => {
  test.setTimeout(180_000);
  const { execFile } = await import("node:child_process");
  const { promisify } = await import("node:util");
  const run = promisify(execFile);
  const result = await run("../scripts/test-transfers", [], {
    timeout: 170_000,
  });
  expect(result.stdout).toContain("test result: ok");
});
