import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 15_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:3000",
    headless: true,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command:
      "cd .. && CAIRN_ADMIN_TOKEN=dev-admin-token BEDROCK_API_KEY=test BEDROCK_MODEL_ID=test AWS_REGION=us-west-2 ./target/debug/cairn-app --port 3000",
    port: 3000,
    reuseExistingServer: true,
    timeout: 15_000,
  },
});
