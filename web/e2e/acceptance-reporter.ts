import type { Reporter, TestCase, TestResult } from "@playwright/test/reporter";

export default class AcceptanceReporter implements Reporter {
  private pending = 0;

  onTestEnd(_test: TestCase, result: TestResult) {
    if (result.status === "skipped") this.pending++;
  }

  async onEnd() {
    if (this.pending > 0) {
      console.log(`Acceptance incomplete: ${this.pending} pending scenarios.`);
      if (process.env.SCT_FULL_ACCEPTANCE === "1") {
        return { status: "failed" as const };
      }
    }
  }
}
