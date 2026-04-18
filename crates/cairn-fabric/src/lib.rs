//! cairn-fabric — bridges cairn-rs to the FlowFabric Valkey-native execution engine.
//!
//! FlowFabric runs all execution state as atomic Valkey FCALLs (Lua functions),
//! giving cairn-rs durable, lease-based task execution with sub-millisecond
//! state transitions, automatic lease renewal, retry scheduling, suspension
//! with signal-driven resume, multi-dimensional budgets, and rate-limiting quotas.
//!
//! # Architecture
//!
//! ```text
//! User API request
//!   → cairn-app (HTTP/SSE)
//!     → cairn-fabric (this crate)
//!       → FlowFabric Valkey FCALL  (execution source of truth)
//!       → cairn-store EventLog     (audit trail + read model sync)
//! ```
//!
//! # Entry point
//!
//! [`FabricServices`] is the single aggregate that wires all Fabric-backed
//! services. Constructed at startup via [`FabricServices::start()`], it holds
//! the Valkey connection, background engine scanners, and every service impl.
//!
//! ```rust,ignore
//! use cairn_fabric::{FabricConfig, FabricServices};
//!
//! let config = FabricConfig::from_env()?;
//! let fabric = FabricServices::start(config, event_log).await?;
//!
//! // Use fabric.runs, fabric.tasks, fabric.budgets, fabric.quotas, etc.
//! // Worker loop:
//! let worker = fabric.worker.connect().await?;
//! while let Some(task) = worker.claim_next().await? {
//!     task.complete_with_result(None).await?;
//! }
//!
//! fabric.shutdown().await;
//! ```
//!
//! # Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`aggregate`] | [`FabricServices`] — single wiring point for all services |
//! | [`services`] | RunService, TaskService, BudgetService, QuotaService impls via FCALL |
//! | [`worker_sdk`] | [`CairnWorker`] / [`CairnTask`] — claim-process-complete loop |
//! | [`boot`] | [`FabricRuntime`] — Valkey connection + engine scanner lifecycle |
//! | [`config`] | [`FabricConfig`] — env-var-driven configuration |
//! | [`id_map`] | Deterministic cairn ID → FlowFabric ID mapping (UUID v5) |
//! | [`state_map`] | FlowFabric PublicState ↔ cairn RunState/TaskState conversion |
//! | [`event_bridge`] | Async bridge: Fabric mutations → cairn-store RuntimeEvents |
//! | [`stream`] | Tool/LLM frame logging via FlowFabric output streams |
//! | [`suspension`] | Typed suspension builders for approval/subagent/tool-result waits |
//! | [`signal_bridge`] | cairn domain events → FlowFabric signal delivery |

pub mod aggregate;
pub mod boot;
pub mod config;
pub mod constants;
pub mod error;
pub mod event_bridge;
pub mod fcall;
pub mod helpers;
pub mod id_map;
pub mod services;
pub mod signal_bridge;
pub mod state_map;
pub mod stream;
pub mod suspension;
pub mod worker_sdk;

pub use aggregate::FabricServices;
pub use boot::FabricRuntime;
pub use config::FabricConfig;
pub use error::FabricError;
pub use worker_sdk::{CairnTask, CairnWorker};
