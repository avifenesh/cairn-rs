# Security Policy

## Supported versions

cairn-rs is pre-1.0. Security fixes land on `main` and are not backported.
Users should track `main` directly or pin a recent tag.

## Reporting a vulnerability

**Do not open a public GitHub issue for security problems.**

Email security reports to **avifenesh@users.noreply.github.com** or open a
[GitHub Security Advisory](https://github.com/avifenesh/cairn-rs/security/advisories/new)
(private disclosure channel).

Include:
- Affected component (crate + file + line when possible)
- Reproduction steps or proof-of-concept
- Impact assessment: who is affected, under what configuration, what data or
  actions are exposed

We aim to acknowledge reports within 72 hours and triage within 7 days.
Coordinated-disclosure timelines are negotiated per report; 90 days is the
default upper bound absent active exploitation.

## Scope

In scope:
- The cairn-rs binary and its HTTP surface
- Included crates under `crates/`
- The embedded operator dashboard (`ui/`)
- Default configuration, the provided Dockerfile, and `docker-compose.yml`

Out of scope:
- Self-hosted deployments where the operator has configured cairn against
  their own risk model (e.g. disabling auth, exposing admin endpoints to the
  internet). We will document hardening guidance but not treat operator
  misconfiguration as a vulnerability unless defaults are the cause.
- External providers (Anthropic, OpenAI, Bedrock, etc.). Report directly
  to those vendors.
- Plugins distributed outside this repository.

## Known security boundaries

- `CAIRN_ADMIN_TOKEN` is the admin authentication for the HTTP surface. Dev
  defaults (`dev-admin-token`, `cairn-demo-token`) must not be used in
  production. Boot logs warn when defaults are active.
- The `insecure-direct-claim` cargo feature exposes FCALL paths that skip
  scheduler admission and budget checks. It is OFF by default and must
  stay OFF in production builds.
- FlowFabric HMAC secret (`CAIRN_FABRIC_WAITPOINT_HMAC_SECRET`) is required
  when the Fabric backend is enabled; boot fails loud if unset.
