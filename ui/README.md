# Cairn UI

This directory contains the React 19 + Vite operator dashboard served by
`cairn-app`.

## What lives here

- `src/` contains the operator UI pages, shared components, hooks, and API client
- `dist/` is the production bundle that `cairn-app` embeds from `ui/dist`
- `e2e/` contains Playwright coverage for browser flows

## Local development

```bash
cd ui
npm install
npm run dev
```

The Vite dev server runs on `http://localhost:5173` and proxies `/health` and
`/v1` requests to `http://localhost:3000`, so the Rust app should be running in
parallel for most UI work.

## Build and test

```bash
npm run build
npm run lint
npm run test:e2e
```

`npm run build` refreshes `ui/dist`, which is what the Rust binary serves in
release mode and embeds for standalone distribution.
