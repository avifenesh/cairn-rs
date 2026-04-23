//! Memory handlers: sources, ingest jobs, channels, search, feedback,
//! diagnostics, deep-search, provenance, related documents, preserved memory,
//! refresh schedules, and quality endpoints.

use axum::{
    extract::rejection::JsonRejection,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use cairn_domain::{
    ChannelId, ChannelRecord, IngestJobId, IngestJobState, KnowledgeDocumentId, ProjectKey,
    SourceId,
};
use cairn_graph::graph_provenance::GraphProvenanceService;
use cairn_graph::projections::{GraphNode, NodeKind};
use cairn_graph::provenance::ProvenanceService;
use cairn_graph::retrieval_projector::RetrievalGraphProjector;
use cairn_graph::{GraphQueryService, TraversalDirection};
use cairn_memory::deep_search::{DeepSearchRequest, DeepSearchService};
use cairn_memory::diagnostics::DiagnosticsService;
use cairn_memory::diagnostics::{IndexStatus, SourceQualityRecord};
use cairn_memory::in_memory::InMemoryDocumentStore;
use cairn_memory::ingest::{DocumentVersionReadModel, IngestRequest, IngestService, SourceType};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};
use cairn_runtime::{ChannelService, IngestJobService};
use std::collections::HashMap;
use std::sync::Arc;

use cairn_api::memory_api::{CreateMemoryRequest, MemoryEndpoints, MemoryItem, MemoryStatus};

use cairn_api::http::ListResponse;

use crate::{
    bad_request_response, graph_trace_snapshot, json_rejection_response, memory_api_error_response,
    runtime_error_response, tenant_scope_mismatch_error, validation_error_response, AppApiError,
    AppSourceMetadata, AppState, OptionalProjectScopedQuery, PendingIngestJobPayload,
    PreservedMemoryListQuery, PreservedMemorySearchParams, ProjectScope, ProjectScopedQuery,
    TenantScope,
};

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct MemorySearchParams {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    query_text: String,
    limit: Option<usize>,
}

impl MemorySearchParams {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateSourceRequest {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    source_id: String,
    name: Option<String>,
    description: Option<String>,
}

impl CreateSourceRequest {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct MemoryIngestRequest {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    source_id: String,
    document_id: String,
    content: String,
    source_type: Option<SourceType>,
}

impl MemoryIngestRequest {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct UpdateSourceRequest {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    name: Option<String>,
    description: Option<String>,
}

impl UpdateSourceRequest {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SourceChunksQuery {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    limit: Option<usize>,
    offset: Option<usize>,
}

impl SourceChunksQuery {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }

    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateIngestJobRequest {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    job_id: String,
    source_id: String,
    content: String,
    source_type: Option<SourceType>,
    document_id: Option<String>,
}

impl CreateIngestJobRequest {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateChannelRequest {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    name: String,
    capacity: u32,
}

impl CreateChannelRequest {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct ChannelListQuery {
    tenant_id: Option<String>,
    workspace_id: Option<String>,
    project_id: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

impl ChannelListQuery {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_deref().unwrap_or("default"),
            self.workspace_id.as_deref().unwrap_or("default"),
            self.project_id.as_deref().unwrap_or("default"),
        )
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }

    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct ChannelMessagesQuery {
    limit: Option<usize>,
}

impl ChannelMessagesQuery {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SendChannelMessageRequest {
    sender_id: String,
    body: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ConsumeChannelMessageRequest {
    consumer_id: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SendChannelMessageResponse {
    message_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct IngestJobListQuery {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    status: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

impl IngestJobListQuery {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).min(500)
    }

    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CompleteIngestJobRequest {
    success: bool,
    error_message: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct FailIngestJobRequest {
    error_message: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SourceDetailResponse {
    source_id: SourceId,
    project: ProjectKey,
    active: bool,
    document_count: u64,
    chunk_count: u64,
    last_ingested_at: Option<u64>,
    name: Option<String>,
    description: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SourceChunkView {
    chunk_id: String,
    text_preview: String,
    credibility_score: Option<f64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct MemoryFeedbackRequest {
    chunk_id: String,
    source_id: String,
    was_used: bool,
    rating: Option<f32>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SourceQualityStatsResponse {
    source_id: SourceId,
    credibility_score: f64,
    total_retrievals: u64,
    avg_rating: Option<f64>,
    chunk_count: u64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct DeepSearchHttpRequest {
    project: DeepSearchProjectRequest,
    query_text: String,
    max_hops: u32,
    per_hop_limit: usize,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct DeepSearchProjectRequest {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
}

impl DeepSearchHttpRequest {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.project.tenant_id.as_str(),
            self.project.workspace_id.as_str(),
            self.project.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[allow(dead_code)]
pub(crate) struct MemoryProvenanceResponse {
    source: Option<GraphNode>,
    document: Option<GraphNode>,
    chunks: Vec<GraphNode>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct MemoryDiagnosticsResponse {
    index_status: IndexStatus,
    sources: Vec<MemoryDiagnosticsSourceView>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct MemoryDiagnosticsSourceView {
    source_id: SourceId,
    project: ProjectKey,
    chunk_count: u64,
    retrieval_count: u64,
    avg_relevance_score: f64,
    avg_rating: Option<f64>,
    freshness_score: f64,
    credibility_score: f64,
    last_ingested: u64,
}

impl From<SourceQualityRecord> for MemoryDiagnosticsSourceView {
    fn from(value: SourceQualityRecord) -> Self {
        Self {
            source_id: value.source_id,
            project: value.project,
            chunk_count: value.total_chunks,
            retrieval_count: value.total_retrievals,
            avg_relevance_score: value.avg_relevance_score,
            avg_rating: Some(value.avg_rating),
            freshness_score: value.freshness_score,
            credibility_score: value.credibility_score,
            last_ingested: value.last_ingested_at,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateRefreshScheduleRequest {
    interval_ms: u64,
    refresh_url: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RefreshScheduleResponse {
    schedule_id: String,
    source_id: String,
    interval_ms: u64,
    last_refresh_ms: Option<u64>,
    enabled: bool,
    refresh_url: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ProcessRefreshResponse {
    processed_count: usize,
    schedule_ids: Vec<String>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

pub(crate) fn source_detail_for(
    state: &Arc<AppState>,
    project: &ProjectKey,
    source_id: &SourceId,
) -> Option<SourceDetailResponse> {
    let summary = state
        .document_store
        .list_sources(project)
        .into_iter()
        .find(|item| item.source_id == *source_id)?;
    let chunk_count = state
        .document_store
        .all_chunks()
        .into_iter()
        .filter(|chunk| chunk.project == *project && chunk.source_id == *source_id)
        .count() as u64;
    let metadata = state
        .source_metadata
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(source_id.as_str())
        .cloned()
        .unwrap_or_default();

    Some(SourceDetailResponse {
        source_id: summary.source_id,
        project: project.clone(),
        active: true,
        document_count: summary.document_count,
        chunk_count,
        last_ingested_at: summary.last_ingested_at_ms,
        name: metadata.name,
        description: metadata.description,
    })
}

pub(crate) fn parse_ingest_job_state(status: &str) -> Option<IngestJobState> {
    match status {
        "pending" => Some(IngestJobState::Pending),
        "processing" => Some(IngestJobState::Processing),
        "completed" => Some(IngestJobState::Completed),
        "failed" => Some(IngestJobState::Failed),
        _ => None,
    }
}

pub(crate) async fn project_source_in_graph(
    state: &Arc<AppState>,
    _project: &ProjectKey,
    source_id: &SourceId,
) -> Result<(), String> {
    let projector = RetrievalGraphProjector::new(state.graph.clone());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    projector
        .on_source_registered(source_id, ts)
        .await
        .map_err(|err| err.to_string())
}

pub(crate) async fn project_document_in_graph(
    state: &Arc<AppState>,
    _project: &ProjectKey,
    source_id: &SourceId,
    document_id: &KnowledgeDocumentId,
    chunk_ids: Vec<String>,
) -> Result<(), String> {
    let projector = RetrievalGraphProjector::new(state.graph.clone());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    projector
        .on_source_registered(source_id, ts)
        .await
        .map_err(|err| err.to_string())?;
    projector
        .on_document_ingested(document_id, source_id, ts)
        .await
        .map_err(|err| err.to_string())?;
    if !chunk_ids.is_empty() {
        projector
            .on_chunks_created(&chunk_ids, document_id, ts)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub(crate) fn exportable_document_by_id(
    store: &InMemoryDocumentStore,
    document_id: &str,
) -> Option<cairn_memory::in_memory::ExportableDocument> {
    store
        .exportable_documents()
        .into_iter()
        .find(|doc| doc.document_id.as_str() == document_id)
}

pub(crate) fn memory_item_from_exportable_document(
    document: &cairn_memory::in_memory::ExportableDocument,
    relationship: Option<String>,
) -> MemoryItem {
    MemoryItem {
        id: document.document_id.to_string(),
        content: document.text.clone(),
        category: Some("graph_related".to_owned()),
        status: MemoryStatus::Accepted,
        source: relationship.or_else(|| Some(document.source_id.to_string())),
        confidence: None,
        created_at: document.created_at.to_string(),
    }
}

pub(crate) async fn channel_for_tenant(
    state: &Arc<AppState>,
    tenant: TenantScope,
    channel_id: &ChannelId,
) -> Result<ChannelRecord, Response> {
    match state.runtime.channels.get(channel_id).await {
        Ok(Some(channel)) => {
            if !tenant.is_admin && channel.project.tenant_id != *tenant.tenant_id() {
                Err(tenant_scope_mismatch_error().into_response())
            } else {
                Ok(channel)
            }
        }
        Ok(None) => Err(
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "channel not found")
                .into_response(),
        ),
        Err(err) => Err(runtime_error_response(err)),
    }
}

// ── Source handlers ──────────────────────────────────────────────────────────

pub(crate) async fn list_sources_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectScopedQuery>,
) -> impl IntoResponse {
    let all = state.document_store.list_sources(&query.project());
    let items: Vec<_> = all
        .into_iter()
        .skip(query.offset())
        .take(query.limit())
        .collect();
    (StatusCode::OK, Json(items))
}

pub(crate) async fn create_source_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSourceRequest>,
) -> impl IntoResponse {
    let project = body.project();
    let source_id = SourceId::new(body.source_id);
    state
        .source_metadata
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            source_id.as_str().to_owned(),
            AppSourceMetadata {
                name: body.name,
                description: body.description,
            },
        );
    let summary = state.document_store.register_source(&project, &source_id);
    if let Err(e) = project_source_in_graph(&state, &project, &source_id).await {
        tracing::warn!("graph projection failed (non-fatal): {e}");
    }
    (StatusCode::CREATED, Json(summary)).into_response()
}

pub(crate) async fn get_source_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ProjectScopedQuery>,
) -> impl IntoResponse {
    let project = query.project();
    let source_id = SourceId::new(id);
    match source_detail_for(&state, &project, &source_id) {
        Some(detail) => (StatusCode::OK, Json(detail)).into_response(),
        None => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "source not found").into_response()
        }
    }
}

pub(crate) async fn update_source_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSourceRequest>,
) -> impl IntoResponse {
    let project = body.project();
    let source_id = SourceId::new(id);
    if source_detail_for(&state, &project, &source_id).is_none() {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "source not found")
            .into_response();
    }

    state
        .source_metadata
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(source_id.as_str().to_owned())
        .and_modify(|entry| {
            entry.name = body.name.clone();
            entry.description = body.description.clone();
        })
        .or_insert(AppSourceMetadata {
            name: body.name,
            description: body.description,
        });

    match source_detail_for(&state, &project, &source_id) {
        Some(detail) => (StatusCode::OK, Json(detail)).into_response(),
        None => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", "source not found").into_response()
        }
    }
}

pub(crate) async fn list_source_chunks_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<SourceChunksQuery>,
) -> impl IntoResponse {
    let project = query.project();
    let source_id = SourceId::new(id);
    if source_detail_for(&state, &project, &source_id).is_none() {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "source not found")
            .into_response();
    }

    let mut chunks: Vec<SourceChunkView> = state
        .document_store
        .all_chunks()
        .into_iter()
        .filter(|chunk| chunk.project == project && chunk.source_id == source_id)
        .map(|chunk| SourceChunkView {
            chunk_id: chunk.chunk_id.to_string(),
            text_preview: chunk.text.chars().take(100).collect(),
            credibility_score: chunk.credibility_score,
        })
        .collect();
    chunks.sort_by_key(|r| r.chunk_id.clone());
    let total = chunks.len();
    let items = chunks
        .into_iter()
        .skip(query.offset())
        .take(query.limit())
        .collect::<Vec<_>>();
    let has_more = total > query.offset().saturating_add(items.len());
    (StatusCode::OK, Json(ListResponse { has_more, items })).into_response()
}

pub(crate) async fn delete_source_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.document_store.deactivate_source(&SourceId::new(id)) {
        (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
    } else {
        AppApiError::new(StatusCode::NOT_FOUND, "not_found", "source not found").into_response()
    }
}

// ── Refresh schedule handlers ────────────────────────────────────────────────

pub(crate) async fn get_source_refresh_schedule_handler(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> impl IntoResponse {
    let sid = cairn_domain::SourceId::new(&source_id);
    match state.document_store.get_refresh_schedule(&sid) {
        Some(schedule) => (
            StatusCode::OK,
            Json(RefreshScheduleResponse {
                schedule_id: schedule.schedule_id,
                source_id: schedule.source_id.as_str().to_owned(),
                interval_ms: schedule.interval_ms,
                last_refresh_ms: schedule.last_refresh_ms,
                enabled: schedule.enabled,
                refresh_url: schedule.refresh_url,
            }),
        )
            .into_response(),
        None => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("no refresh schedule for source {source_id}"),
        )
        .into_response(),
    }
}

pub(crate) async fn create_source_refresh_schedule_handler(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Query(query): Query<OptionalProjectScopedQuery>,
    Json(body): Json<CreateRefreshScheduleRequest>,
) -> impl IntoResponse {
    let sid = cairn_domain::SourceId::new(&source_id);
    let project = query.project();
    let schedule = state.document_store.create_refresh_schedule(
        &sid,
        &project,
        body.interval_ms,
        body.refresh_url,
    );
    (
        StatusCode::OK,
        Json(RefreshScheduleResponse {
            schedule_id: schedule.schedule_id,
            source_id: schedule.source_id.as_str().to_owned(),
            interval_ms: schedule.interval_ms,
            last_refresh_ms: schedule.last_refresh_ms,
            enabled: schedule.enabled,
            refresh_url: schedule.refresh_url,
        }),
    )
        .into_response()
}

pub(crate) async fn process_source_refresh_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let due = state.document_store.list_due_schedules(now);
    let ids: Vec<String> = due.iter().map(|s| s.schedule_id.clone()).collect();
    let count = ids.len();
    for schedule in &due {
        state
            .document_store
            .update_last_refresh_ms(&schedule.schedule_id, now);
    }
    (
        StatusCode::OK,
        Json(ProcessRefreshResponse {
            processed_count: count,
            schedule_ids: ids,
        }),
    )
        .into_response()
}

// ── Quality handler ──────────────────────────────────────────────────────────

pub(crate) async fn source_quality_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.diagnostics.source_quality(&SourceId::new(id)).await {
        Ok(Some(record)) => (
            StatusCode::OK,
            Json(SourceQualityStatsResponse {
                source_id: record.source_id,
                credibility_score: record.credibility_score,
                total_retrievals: record.total_retrievals,
                avg_rating: Some(record.avg_rating),
                chunk_count: record.total_chunks,
            }),
        )
            .into_response(),
        Ok(None) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "source quality not found",
        )
        .into_response(),
        Err(err) => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            err.to_string(),
        )
        .into_response(),
    }
}

// ── Search & feedback handlers ───────────────────────────────────────────────

pub(crate) async fn memory_search_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MemorySearchParams>,
) -> impl IntoResponse {
    match state
        .retrieval
        .query(RetrievalQuery {
            project: query.project(),
            query_text: query.query_text,
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: query.limit.unwrap_or(10),
            metadata_filters: Vec::new(),
            scoring_policy: None,
        })
        .await
    {
        Ok(response) => {
            for result in &response.results {
                state.diagnostics.record_retrieval_hit(
                    &result.chunk.source_id,
                    result.breakdown.lexical_relevance.max(result.score),
                );
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "results": response.results,
                    "diagnostics": response.diagnostics,
                })),
            )
                .into_response()
        }
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn memory_feedback_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MemoryFeedbackRequest>,
) -> impl IntoResponse {
    let rating_f64 = body.rating.map(|r| r as f64);

    state.diagnostics.record_retrieval_feedback(
        &SourceId::new(&body.source_id),
        &body.chunk_id,
        body.was_used,
        rating_f64,
    );

    // When a positive rating is provided, update the chunk's credibility_score
    // so that subsequent retrievals benefit from the boosted signal.
    if let Some(rating) = rating_f64 {
        if rating > 0.0 {
            let normalised = (rating / 5.0).clamp(0.0, 1.0);
            let mut chunks = state.document_store.chunks_mut();
            for chunk in chunks.iter_mut() {
                if chunk.chunk_id.as_str() == body.chunk_id {
                    let prev = chunk.credibility_score.unwrap_or(0.5);
                    // Running average between existing score and new rating.
                    chunk.credibility_score = Some((prev + normalised) / 2.0);
                    break;
                }
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ── Document handlers ────────────────────────────────────────────────────────

pub(crate) async fn get_memory_document_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let document_id = KnowledgeDocumentId::new(id);
    let Some(document) = exportable_document_by_id(&state.document_store, document_id.as_str())
    else {
        return AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "memory document not found",
        )
        .into_response();
    };

    match <dyn DocumentVersionReadModel>::list_versions(
        state.document_store.as_ref(),
        &document_id,
        1,
    )
    .await
    {
        Ok(versions) => {
            let chunk_count = state
                .document_store
                .all_current_chunks()
                .into_iter()
                .filter(|chunk| chunk.document_id == document_id)
                .count();
            match versions.into_iter().next() {
                Some(version) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "document_id": document.document_id,
                        "source_id": document.source_id,
                        "version": version.version,
                        "content_hash": version.content_hash,
                        "ingested_at_ms": version.ingested_at_ms,
                        "chunk_count": chunk_count,
                    })),
                )
                    .into_response(),
                None => AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "memory document version not found",
                )
                .into_response(),
            }
        }
        Err(err) => {
            tracing::error!("memory document version lookup failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal error",
            )
            .into_response()
        }
    }
}

pub(crate) async fn list_memory_document_versions_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let document_id = KnowledgeDocumentId::new(id);
    match <dyn DocumentVersionReadModel>::list_versions(
        state.document_store.as_ref(),
        &document_id,
        100,
    )
    .await
    {
        Ok(versions) => (StatusCode::OK, Json(versions)).into_response(),
        Err(err) => {
            tracing::error!("list_memory_document_versions failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        }
    }
}

// ── Ingest handlers ──────────────────────────────────────────────────────────

pub(crate) async fn memory_ingest_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MemoryIngestRequest>,
) -> impl IntoResponse {
    // Validate required ids up front (closes #238). Previously an empty
    // source_id silently minted a blank-id source; malformed JSON under
    // a structured source_type was accepted and only failed later at
    // retrieval time.
    if body.source_id.trim().is_empty() {
        return bad_request_response("source_id must not be empty");
    }
    if body.document_id.trim().is_empty() {
        return bad_request_response("document_id must not be empty");
    }
    // Structured-JSON source types must carry a parseable JSON payload.
    // Covers both `structured_json` and `json_structured` aliases;
    // rejecting here surfaces the operator's mistake as a 422 instead
    // of a late chunker failure.
    if matches!(
        body.source_type,
        Some(SourceType::StructuredJson) | Some(SourceType::JsonStructured)
    ) {
        if let Err(err) = serde_json::from_str::<serde_json::Value>(&body.content) {
            return bad_request_response(format!(
                "content is not valid JSON for structured source_type: {err}"
            ));
        }
    }

    let project = body.project();
    let source_id = SourceId::new(body.source_id);
    let document_id = KnowledgeDocumentId::new(body.document_id);

    state.document_store.register_source(&project, &source_id);

    match state
        .ingest
        .submit(IngestRequest {
            document_id: document_id.clone(),
            source_id: source_id.clone(),
            source_type: body.source_type.unwrap_or(SourceType::PlainText),
            project: project.clone(),
            content: body.content,
            tags: Vec::new(),
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
    {
        Ok(()) => {
            let chunks: Vec<_> = state
                .document_store
                .all_current_chunks()
                .into_iter()
                .filter(|chunk| chunk.document_id == document_id)
                .collect();
            let chunk_count = chunks.len() as u64;
            state
                .diagnostics
                .record_ingest(&source_id, &project, chunk_count);
            if let Err(e) = project_document_in_graph(
                &state,
                &project,
                &source_id,
                &document_id,
                chunks
                    .iter()
                    .map(|chunk| chunk.chunk_id.as_str().to_owned())
                    .collect(),
            )
            .await
            {
                tracing::warn!("graph projection failed (non-fatal): {e}");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "document_id": document_id,
                    "source_id": source_id,
                    "chunk_count": chunk_count,
                })),
            )
                .into_response()
        }
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn create_ingest_job_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateIngestJobRequest>,
) -> impl IntoResponse {
    let project = body.project();
    let source_id = SourceId::new(body.source_id);
    let job_id = IngestJobId::new(body.job_id);
    let document_id = KnowledgeDocumentId::new(
        body.document_id
            .unwrap_or_else(|| format!("doc_{}", job_id.as_str())),
    );

    state.document_store.register_source(&project, &source_id);
    if let Err(e) = project_source_in_graph(&state, &project, &source_id).await {
        tracing::warn!("graph projection failed (non-fatal): {e}");
    }
    state
        .pending_ingest_jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            job_id.as_str().to_owned(),
            PendingIngestJobPayload {
                project: project.clone(),
                source_id: source_id.clone(),
                document_id,
                content: body.content,
                source_type: body.source_type.unwrap_or(SourceType::PlainText),
            },
        );

    let response = match state
        .runtime
        .ingest_jobs
        .start(&project, job_id.clone(), Some(source_id.clone()), 1)
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    };

    if response.status() != StatusCode::CREATED {
        state
            .pending_ingest_jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(job_id.as_str());
    }

    response
}

pub(crate) async fn get_ingest_job_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.runtime.ingest_jobs.get(&IngestJobId::new(id)).await {
        Ok(Some(record)) => (StatusCode::OK, Json(record)).into_response(),
        Ok(None) => AppApiError::new(StatusCode::NOT_FOUND, "not_found", "ingest job not found")
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_ingest_jobs_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<IngestJobListQuery>,
) -> impl IntoResponse {
    let project = query.project();
    match state
        .runtime
        .ingest_jobs
        .list_by_project(&project, query.limit(), query.offset())
        .await
    {
        Ok(mut records) => {
            if let Some(status) = query.status.as_deref() {
                let Some(expected) = parse_ingest_job_state(status) else {
                    return validation_error_response("invalid ingest job status filter");
                };
                records.retain(|record| record.state == expected);
            }
            (StatusCode::OK, Json(records)).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn complete_ingest_job_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CompleteIngestJobRequest>,
) -> impl IntoResponse {
    let job_id = IngestJobId::new(id);
    let job = match state.runtime.ingest_jobs.get(&job_id).await {
        Ok(Some(job)) => job,
        Ok(None) => {
            return AppApiError::new(StatusCode::NOT_FOUND, "not_found", "ingest job not found")
                .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    if body.success {
        let pending = state
            .pending_ingest_jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(job_id.as_str())
            .cloned();
        if let Some(pending) = pending {
            if let Err(err) = state
                .ingest
                .submit(IngestRequest {
                    document_id: pending.document_id.clone(),
                    source_id: pending.source_id.clone(),
                    source_type: pending.source_type,
                    project: pending.project.clone(),
                    content: pending.content.clone(),
                    tags: Vec::new(),
                    corpus_id: None,
                    bundle_source_id: None,
                    import_id: None,
                })
                .await
            {
                return AppApiError::new(StatusCode::BAD_REQUEST, "ingest_failed", err.to_string())
                    .into_response();
            }

            let chunks: Vec<_> = state
                .document_store
                .all_chunks()
                .into_iter()
                .filter(|chunk| chunk.document_id == pending.document_id)
                .collect();
            state.diagnostics.record_ingest(
                &pending.source_id,
                &pending.project,
                chunks.len() as u64,
            );
            if let Err(e) = project_document_in_graph(
                &state,
                &pending.project,
                &pending.source_id,
                &pending.document_id,
                chunks
                    .iter()
                    .map(|chunk| chunk.chunk_id.as_str().to_owned())
                    .collect(),
            )
            .await
            {
                tracing::warn!("graph projection failed (non-fatal): {e}");
            }
        }
    }

    let response = match state
        .runtime
        .ingest_jobs
        .complete(
            &job.project,
            job_id.clone(),
            body.success,
            body.error_message.clone(),
        )
        .await
    {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    };

    state
        .pending_ingest_jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(job_id.as_str());
    response
}

pub(crate) async fn fail_ingest_job_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<FailIngestJobRequest>,
) -> impl IntoResponse {
    complete_ingest_job_handler(
        State(state),
        Path(id),
        Json(CompleteIngestJobRequest {
            success: false,
            error_message: Some(body.error_message),
        }),
    )
    .await
}

// ── Channel handlers ─────────────────────────────────────────────────────────

pub(crate) async fn create_channel_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateChannelRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .channels
        .create(&body.project(), body.name, body.capacity)
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_channels_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ChannelListQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .channels
        .list_channels(&query.project(), query.limit(), query.offset())
        .await
    {
        Ok(items) => {
            let has_more = items.len() == query.limit();
            (StatusCode::OK, Json(ListResponse { items, has_more })).into_response()
        }
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn send_channel_message_handler(
    State(state): State<Arc<AppState>>,
    tenant: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<SendChannelMessageRequest>,
) -> impl IntoResponse {
    let channel_id = ChannelId::new(id);
    let channel = match channel_for_tenant(&state, tenant, &channel_id).await {
        Ok(channel) => channel,
        Err(response) => return response,
    };

    match state
        .runtime
        .channels
        .send(&channel.channel_id, body.sender_id, body.body)
        .await
    {
        Ok(message_id) => (
            StatusCode::OK,
            Json(SendChannelMessageResponse { message_id }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn consume_channel_message_handler(
    State(state): State<Arc<AppState>>,
    tenant: TenantScope,
    Path(id): Path<String>,
    Json(body): Json<ConsumeChannelMessageRequest>,
) -> impl IntoResponse {
    let channel_id = ChannelId::new(id);
    let channel = match channel_for_tenant(&state, tenant, &channel_id).await {
        Ok(channel) => channel,
        Err(response) => return response,
    };

    match state
        .runtime
        .channels
        .consume(&channel.channel_id, body.consumer_id)
        .await
    {
        Ok(message) => (StatusCode::OK, Json(message)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_channel_messages_handler(
    State(state): State<Arc<AppState>>,
    tenant: TenantScope,
    Path(id): Path<String>,
    Query(query): Query<ChannelMessagesQuery>,
) -> impl IntoResponse {
    let channel_id = ChannelId::new(id);
    let channel = match channel_for_tenant(&state, tenant, &channel_id).await {
        Ok(channel) => channel,
        Err(response) => return response,
    };

    match state
        .runtime
        .channels
        .list_messages(&channel.channel_id, query.limit())
        .await
    {
        Ok(messages) => (StatusCode::OK, Json(messages)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

// ── Diagnostics handlers ────────────────────────────────────────────────────

pub(crate) async fn memory_diagnostics_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectScopedQuery>,
) -> impl IntoResponse {
    let project = query.project();
    match (
        state.diagnostics.index_status(&project).await,
        state
            .diagnostics
            .list_source_quality(&project, query.limit.unwrap_or(100))
            .await,
    ) {
        (Ok(index_status), Ok(sources)) => (
            StatusCode::OK,
            Json(MemoryDiagnosticsResponse {
                index_status,
                sources: sources.into_iter().map(Into::into).collect(),
            }),
        )
            .into_response(),
        (Err(err), _) | (_, Err(err)) => (AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            err.to_string(),
        ))
        .into_response(),
    }
}

pub(crate) async fn memory_deep_search_handler(
    State(state): State<Arc<AppState>>,
    body: Result<Json<DeepSearchHttpRequest>, JsonRejection>,
) -> impl IntoResponse {
    let Json(body) = match body {
        Ok(body) => body,
        Err(err) => return json_rejection_response(err),
    };

    match state
        .deep_search
        .search(DeepSearchRequest {
            project: body.project(),
            query_text: body.query_text,
            max_hops: body.max_hops,
            per_hop_limit: body.per_hop_limit,
            mode: RetrievalMode::LexicalOnly,
        })
        .await
    {
        Ok(response) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "hops": response.hops,
                "merged_results": response.merged_results,
                "total_latency_ms": response.total_latency_ms,
            })),
        )
            .into_response(),
        Err(err) => AppApiError::new(
            StatusCode::BAD_REQUEST,
            "deep_search_failed",
            err.to_string(),
        )
        .into_response(),
    }
}

pub(crate) async fn memory_provenance_handler(
    State(state): State<Arc<AppState>>,
    Path(document_id): Path<String>,
) -> impl IntoResponse {
    let provenance = GraphProvenanceService::new(state.graph.clone());
    let chain = match provenance.provenance_chain(&document_id, 5).await {
        Ok(chain) => chain,
        Err(err) => {
            return AppApiError::new(
                StatusCode::BAD_REQUEST,
                "provenance_failed",
                err.to_string(),
            )
            .into_response();
        }
    };

    let nodes = state.graph.all_nodes();
    let mut chunk_nodes = state
        .graph
        .neighbors(
            &document_id,
            Some(cairn_graph::EdgeKind::EmbeddedAs),
            TraversalDirection::Downstream,
            256,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(_, node)| node)
        .collect::<Vec<_>>();
    chunk_nodes.sort_by_key(|r| r.node_id.clone());

    let source = chain
        .links
        .iter()
        .filter_map(|link| nodes.get(&link.node_id))
        .find(|node| node.kind == NodeKind::Source)
        .cloned();

    (
        StatusCode::OK,
        Json(MemoryProvenanceResponse {
            source,
            document: nodes.get(&document_id).cloned(),
            chunks: chunk_nodes,
        }),
    )
        .into_response()
}

pub(crate) async fn memory_related_documents_handler(
    State(state): State<Arc<AppState>>,
    Path(document_id): Path<String>,
) -> impl IntoResponse {
    let _seed = match exportable_document_by_id(&state.document_store, &document_id) {
        Some(document) => document,
        None => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "document_not_found",
                "document not found",
            )
            .into_response();
        }
    };

    // Query the graph for document nodes linked to this seed document.
    let neighbors = state
        .graph
        .neighbors(&document_id, None, TraversalDirection::Upstream, 20)
        .await
        .unwrap_or_default();

    let documents = state.document_store.exportable_documents();
    let by_id: HashMap<String, cairn_memory::in_memory::ExportableDocument> = documents
        .into_iter()
        .map(|doc| (doc.document_id.to_string(), doc))
        .collect();

    let items = neighbors
        .into_iter()
        .filter_map(|(edge, node)| {
            let doc = by_id.get(&node.node_id)?;
            let relationship = Some(format!("{:?}", edge.kind).to_lowercase());
            Some(memory_item_from_exportable_document(doc, relationship))
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(items)).into_response()
}

// ── Preserved memory handlers ────────────────────────────────────────────────

pub(crate) async fn list_memories_preserved_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectScope<PreservedMemoryListQuery>,
) -> impl IntoResponse {
    let query = project_scope.into_inner();
    match state
        .memory_api
        .list(&query.project(), &query.list_query())
        .await
    {
        Ok(list) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "items": list.items,
                "hasMore": list.has_more,
                "has_more": list.has_more,
            })),
        )
            .into_response(),
        Err(err) => memory_api_error_response(err),
    }
}

pub(crate) async fn search_memories_preserved_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectScope<PreservedMemorySearchParams>,
) -> impl IntoResponse {
    let query = project_scope.into_inner();
    if query.q.trim().is_empty() {
        return bad_request_response("query parameter q is required");
    }

    match state
        .memory_api
        .search(&query.project(), &query.search_query())
        .await
    {
        Ok(items) => (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response(),
        Err(err) => memory_api_error_response(err),
    }
}

pub(crate) async fn create_memory_preserved_handler(
    State(state): State<Arc<AppState>>,
    project_scope: ProjectScope<OptionalProjectScopedQuery>,
    Json(body): Json<CreateMemoryRequest>,
) -> impl IntoResponse {
    let scope = project_scope.into_inner();
    match state.memory_api.create(&scope.project(), &body).await {
        Ok(item) => (StatusCode::CREATED, Json(item)).into_response(),
        Err(err) => memory_api_error_response(err),
    }
}

pub(crate) async fn accept_memory_preserved_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    project_scope: ProjectScope<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    let scope = project_scope.into_inner();
    match state.memory_api.accept(&scope.project(), &id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(err) => memory_api_error_response(err),
    }
}

pub(crate) async fn reject_memory_preserved_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    project_scope: ProjectScope<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    let scope = project_scope.into_inner();
    match state.memory_api.reject(&scope.project(), &id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(err) => memory_api_error_response(err),
    }
}

pub(crate) async fn graph_trace_preserved_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    let response = graph_trace_snapshot(
        state.graph.as_ref(),
        &query.project(),
        query.limit().clamp(100, 500),
    );
    (StatusCode::OK, Json(response)).into_response()
}
