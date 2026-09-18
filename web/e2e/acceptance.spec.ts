import { test } from "@playwright/test";

// Milestone gates, not passing tests. Expand these into real API/SDK/browser
// workflows using ARCHITECTURE_PLAN.md §16 as each capability lands.
for (const scenario of [
  "M2: resume mirrored uploads after restart and verify downloaded bytes",
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
