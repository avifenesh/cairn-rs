use std::collections::HashMap;
use std::sync::Mutex;

use cairn_domain::{EvalDataset, EvalDatasetEntry, EvalSubjectKind, TenantId};

#[derive(Debug)]
pub enum EvalDatasetError {
    NotFound(String),
}

impl std::fmt::Display for EvalDatasetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalDatasetError::NotFound(id) => write!(f, "eval dataset not found: {id}"),
        }
    }
}

impl std::error::Error for EvalDatasetError {}

struct DatasetState {
    datasets: HashMap<String, EvalDataset>,
}

pub struct EvalDatasetServiceImpl {
    state: Mutex<DatasetState>,
}

impl EvalDatasetServiceImpl {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(DatasetState {
                datasets: HashMap::new(),
            }),
        }
    }

    pub fn create(
        &self,
        tenant_id: TenantId,
        name: String,
        subject_kind: EvalSubjectKind,
    ) -> EvalDataset {
        let dataset_id = format!("dataset_{}", now_millis());
        let dataset = EvalDataset {
            dataset_id: dataset_id.clone(),
            tenant_id,
            name,
            subject_kind,
            entries: Vec::new(),
            created_at_ms: now_millis(),
        };
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .datasets
            .insert(dataset_id, dataset.clone());
        dataset
    }

    pub fn add_entry(
        &self,
        dataset_id: &str,
        input: serde_json::Value,
        expected_output: Option<serde_json::Value>,
        tags: Vec<String>,
    ) -> Result<EvalDataset, EvalDatasetError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let dataset = state
            .datasets
            .get_mut(dataset_id)
            .ok_or_else(|| EvalDatasetError::NotFound(dataset_id.to_owned()))?;
        dataset.entries.push(EvalDatasetEntry {
            input,
            expected_output,
            tags,
        });
        Ok(dataset.clone())
    }

    pub fn get(&self, dataset_id: &str) -> Option<EvalDataset> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .datasets
            .get(dataset_id)
            .cloned()
    }

    pub fn list(&self, tenant_id: &TenantId) -> Vec<EvalDataset> {
        let mut datasets = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .datasets
            .values()
            .filter(|dataset| dataset.tenant_id == *tenant_id)
            .cloned()
            .collect::<Vec<_>>();
        datasets.sort_by_key(|dataset| (dataset.created_at_ms, dataset.dataset_id.clone()));
        datasets
    }
}

impl Default for EvalDatasetServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_create_add_get_list_round_trip() {
        let svc = EvalDatasetServiceImpl::new();
        let tenant_id = TenantId::new("tenant_dataset");
        let dataset = svc.create(
            tenant_id.clone(),
            "Prompt Dataset".to_owned(),
            EvalSubjectKind::PromptRelease,
        );
        let updated = svc
            .add_entry(
                &dataset.dataset_id,
                serde_json::json!({"input": "hello"}),
                Some(serde_json::json!({"output": "world"})),
                vec!["smoke".to_owned()],
            )
            .unwrap();

        assert_eq!(updated.entries.len(), 1);
        assert_eq!(svc.get(&dataset.dataset_id).unwrap().entries.len(), 1);
        assert_eq!(svc.list(&tenant_id).len(), 1);
    }
}
