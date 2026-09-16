import { defineConfig } from "@playwright/test";

const port = Number(process.env.SCT_E2E_PORT ?? "18080");
const origin = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: true,
  retries: 0,
  reporter: [["list"], ["./e2e/acceptance-reporter.ts"]],
  use: { baseURL: origin, browserName: "chromium", trace: "retain-on-failure" },
  webServer: {
    command: "../target/debug/sct-server",
    env: { SCT_BIND: `127.0.0.1:${port}`, SCT_WEB_DIR: "dist" },
    url: `${origin}/health/live`,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
