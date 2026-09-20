import { expect, test } from "@playwright/test";
import { operationsWorkflow } from "./operations";

// Milestone gates, not passing tests. Expand these into real API/SDK/browser
// workflows using ARCHITECTURE_PLAN.md §16 as each capability lands.
for (const scenario of [
  "M5: qualify scale, real backend failures and independent SDK distribution",
]) {
  test.fixme(scenario, async () => {
    throw new Error(
      "Pending implementation; replace with real acceptance assertions",
    );
  });
}

// M2 is an API/SDK capability. Its real-placement harness opens actual HTTP
// connections; programs own backup mutations.
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

test("M3: publish atomic trees and enforce retention with shared staging limits", async () => {
  test.setTimeout(360_000);
  const { execFile } = await import("node:child_process");
  const { promisify } = await import("node:util");
  const result = await promisify(execFile)("../scripts/test-trees", [], {
    timeout: 350_000,
  });
  expect(result.stdout.match(/M3 tree lifecycle passed/g)).toHaveLength(5);
  expect(result.stdout).toContain("test result: ok");
});

test("M4: administer storage in the browser and recover an isolated checkpoint", async ({
  page,
  request,
}) => {
  test.setTimeout(240_000);
  await operationsWorkflow(page, request);
  const { execFile } = await import("node:child_process");
  const { promisify } = await import("node:util");
  const result = await promisify(execFile)("../scripts/test-m4", [], {
    timeout: 180_000,
  });
  expect(result.stdout).toContain("M4 independent recovery qualified");
  expect(result.stdout).toContain("test result: ok");
});
