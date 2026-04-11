/**
 * Shared E2E test helpers — single source of truth for signIn, nav, API helpers.
 *
 * Both operator-journey.spec.ts and real-scenarios.spec.ts import from here.
 */
import { expect, type Page, type APIRequestContext } from "@playwright/test";

export const TOKEN = "dev-admin-token";
export const BASE = "http://localhost:3000";
export const HDR = { Authorization: `Bearer ${TOKEN}`, "Content-Type": "application/json" };
export const DEFAULT_SCOPE = {
  tenant_id: "default_tenant",
  workspace_id: "default_workspace",
  project_id: "default_project",
};

// ── Auth ─────────────────────────────────────────────────────────────────────

export async function signIn(page: Page) {
  await page.goto("/");
  await page.waitForLoadState("domcontentloaded");
  const sidebar = page.getByTestId("sidebar");

  // Already signed in (localStorage token persisted from prior test in same context)
  if (await sidebar.isVisible({ timeout: 2000 }).catch(() => false)) return;

  const tokenInput = page.getByTestId("login-token-input");
  if (!(await tokenInput.isVisible({ timeout: 3000 }).catch(() => false))) return;

  await tokenInput.fill(TOKEN);
  // Wait for React to enable the submit button
  const submitBtn = page.getByTestId("login-submit-btn");
  await expect(submitBtn).toBeEnabled({ timeout: 3000 });
  await submitBtn.click({ timeout: 5000 });
  await expect(sidebar).toBeVisible({ timeout: 10_000 });
}

// ── Navigation ───────────────────────────────────────────────────────────────

export async function nav(page: Page, hash: string) {
  await page.goto(`/#${hash}`);
  await page.waitForLoadState("domcontentloaded");
  // Wait for the page content to render (sidebar visible = app is mounted)
  await page.getByTestId("sidebar").waitFor({ state: "visible", timeout: 5000 }).catch(() => {});
}

// ── API helpers ──────────────────────────────────────────────────────────────

export async function apiPost(r: APIRequestContext, path: string, data: object) {
  const resp = await r.post(`${BASE}${path}`, { headers: HDR, data });
  return { status: resp.status(), body: await resp.json().catch(() => ({})) };
}

export async function apiGet(r: APIRequestContext, path: string) {
  const resp = await r.get(`${BASE}${path}`, { headers: { Authorization: `Bearer ${TOKEN}` } });
  return { status: resp.status(), body: await resp.json().catch(() => ({})) };
}

export async function apiPut(r: APIRequestContext, path: string, data: object) {
  const resp = await r.put(`${BASE}${path}`, { headers: HDR, data });
  return { status: resp.status(), body: await resp.json().catch(() => ({})) };
}

export async function apiDel(r: APIRequestContext, path: string) {
  return r.delete(`${BASE}${path}`, { headers: { Authorization: `Bearer ${TOKEN}` } });
}

// ── Data extraction ──────────────────────────────────────────────────────────

/** Extract array from various API response shapes. */
export function listFrom<T = unknown>(resp: unknown): T[] {
  const b = (resp as Record<string, unknown>)?.body ?? resp;
  if (Array.isArray(b)) return b as T[];
  const obj = b as Record<string, unknown>;
  for (const key of ["items", "events", "results", "matches", "data"]) {
    if (Array.isArray(obj?.[key])) return obj[key] as T[];
  }
  return [];
}

// ── ID generation ────────────────────────────────────────────────────────────

export const uid = () => Date.now().toString(36) + Math.random().toString(36).slice(2, 6);

// ── LLM test tracking ───────────────────────────────────────────────────────

let _llmAssertionsExercised = 0;

/** Call inside if(status===200) blocks to count real LLM assertions. */
export function trackLlmAssertion() { _llmAssertionsExercised++; }

/** Get count of real LLM assertions exercised in this worker. */
export function llmAssertionCount() { return _llmAssertionsExercised; }
