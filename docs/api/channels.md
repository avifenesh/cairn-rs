# Channels, Mailbox, Signals, Feed, Notifications

Outbound messaging: notification channels (Slack/email/webhook), mailbox, signals subscriptions, operator notifications, and the activity feed.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 19**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/channels` | Preserve | query: limit?; { items } |
| `POST` | `/v1/channels` | Preserve |  |
| `POST` | `/v1/channels/:id/consume` | Preserve |  |
| `GET` | `/v1/channels/:id/messages` | Preserve |  |
| `POST` | `/v1/channels/:id/send` | Preserve |  |
| `GET` | `/v1/feed` | Preserve | query: limit?, before?, source?, unread?; { items, hasMore } |
| `POST` | `/v1/feed/:id/read` | Preserve | path param: id; { ok } |
| `POST` | `/v1/feed/read-all` | Preserve | { changed } |
| `GET` | `/v1/mailbox` | Preserve |  |
| `POST` | `/v1/mailbox` | Preserve |  |
| `DELETE` | `/v1/mailbox/:id` | Preserve |  |
| `GET` | `/v1/notifications` | Preserve |  |
| `POST` | `/v1/notifications/:id/read` | Preserve |  |
| `POST` | `/v1/notifications/read-all` | Preserve |  |
| `GET` | `/v1/signals` | Preserve |  |
| `POST` | `/v1/signals` | Preserve |  |
| `GET` | `/v1/signals/subscriptions` | Preserve |  |
| `POST` | `/v1/signals/subscriptions` | Preserve |  |
| `DELETE` | `/v1/signals/subscriptions/:id` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
