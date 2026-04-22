# API Quick Reference (moved)

The per-subject HTTP reference now lives at [`docs/api/`](./api/README.md). Each subject file lists method, path, classification (Preserve / Transitional / IntentionallyBroken), and the minimum-contract note from the compat catalog.

Source of truth for the full route inventory: [`tests/compat/http_routes.tsv`](../tests/compat/http_routes.tsv), enforced against the live router by `cargo test -p cairn-api --test compat_catalog_sync`. Coverage of the TSV by the per-subject docs is enforced by `cargo test -p cairn-api --test api_docs_coverage`.

Start at [`docs/api/README.md`](./api/README.md) for the subject index.
