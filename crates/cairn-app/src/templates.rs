//! RFC 012 onboarding starter templates.
//!
//! Provides a `TemplateRegistry` with built-in templates for common agent
//! patterns. Each template includes file stubs (system prompts, eval suites,
//! provider configs) that can be applied to a project with a single POST.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

// ── Types ────────────────────────────────────────────────────────────────────

/// Template category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateCategory {
    Chatbot,
    CodeAssistant,
    DataPipeline,
    CustomerSupport,
}

/// One file within a template.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateFile {
    pub path: String,
    pub description: String,
    pub content: String,
}

/// A starter template that can be applied to a project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: TemplateCategory,
    pub files: Vec<TemplateFile>,
}

/// Summary returned by the list endpoint (no file contents).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: TemplateCategory,
    pub file_count: usize,
}

impl From<&Template> for TemplateSummary {
    fn from(t: &Template) -> Self {
        Self {
            id: t.id.clone(),
            name: t.name.clone(),
            description: t.description.clone(),
            category: t.category,
            file_count: t.files.len(),
        }
    }
}

/// Result of applying a template to a project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyResult {
    pub template_id: String,
    pub project_id: String,
    pub files_created: Vec<String>,
}

/// Request body for the apply endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct ApplyRequest {
    pub project_id: String,
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// In-memory registry of starter templates.
///
/// Seeded at startup with built-in templates. Custom templates can be
/// added at runtime.
pub struct TemplateRegistry {
    templates: HashMap<String, Template>,
}

impl TemplateRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
        }
    }

    /// Create a registry pre-seeded with the built-in templates.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        for t in builtin_templates() {
            reg.register(t);
        }
        reg
    }

    /// Register a template (overwrites if the ID already exists).
    pub fn register(&mut self, template: Template) {
        self.templates.insert(template.id.clone(), template);
    }

    /// List all templates (summaries only, no file contents).
    pub fn list(&self) -> Vec<TemplateSummary> {
        let mut summaries: Vec<TemplateSummary> =
            self.templates.values().map(TemplateSummary::from).collect();
        summaries.sort_by(|a, b| a.id.cmp(&b.id));
        summaries
    }

    /// Get a template by ID (full detail with file contents).
    pub fn get(&self, id: &str) -> Option<&Template> {
        self.templates.get(id)
    }

    /// Apply a template to a project (returns list of created file paths).
    pub fn apply(&self, template_id: &str, project_id: &str) -> Option<ApplyResult> {
        let template = self.templates.get(template_id)?;
        let files_created: Vec<String> = template
            .files
            .iter()
            .map(|f| format!("projects/{}/{}", project_id, f.path))
            .collect();
        Some(ApplyResult {
            template_id: template_id.to_owned(),
            project_id: project_id.to_owned(),
            files_created,
        })
    }

    /// Number of registered templates.
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

// ── Built-in templates ───────────────────────────────────────────────────────

/// Return the three built-in starter templates.
pub fn builtin_templates() -> Vec<Template> {
    vec![
        simple_chatbot_template(),
        code_reviewer_template(),
        data_analyst_template(),
    ]
}

fn simple_chatbot_template() -> Template {
    Template {
        id: "simple-chatbot".into(),
        name: "Simple Chatbot".into(),
        description: "Basic Q&A agent with a system prompt. Good starting point for \
                      conversational assistants."
            .into(),
        category: TemplateCategory::Chatbot,
        files: vec![
            TemplateFile {
                path: "prompts/system.md".into(),
                description: "System prompt for the chatbot".into(),
                content: indoc(
                    "You are a helpful assistant. Answer questions clearly and concisely.\n\
                     If you don't know the answer, say so honestly.\n\
                     Always be polite and professional.",
                ),
            },
            TemplateFile {
                path: "config/session.json".into(),
                description: "Default session configuration".into(),
                content: indoc(
                    "{\n  \"max_turns\": 50,\n  \"timeout_ms\": 30000,\n  \
                     \"temperature_milli\": 700\n}",
                ),
            },
            TemplateFile {
                path: "config/provider.json".into(),
                description: "Provider binding configuration".into(),
                content: indoc(
                    "{\n  \"provider_family\": \"openai\",\n  \
                     \"model_id\": \"gpt-4o\",\n  \
                     \"operation\": \"generate\"\n}",
                ),
            },
            TemplateFile {
                path: "evals/basic_qa.json".into(),
                description: "Basic Q&A eval suite stub".into(),
                content: indoc(
                    "{\n  \"suite\": \"basic_qa\",\n  \"evaluator\": \"exact_match\",\n  \
                     \"cases\": [\n    {\n      \"input\": \"What is 2+2?\",\n      \
                     \"expected\": \"4\"\n    }\n  ]\n}",
                ),
            },
        ],
    }
}

fn code_reviewer_template() -> Template {
    Template {
        id: "code-reviewer".into(),
        name: "Code Reviewer".into(),
        description: "Code review agent with eval criteria for review quality. \
                      Analyzes diffs and produces structured feedback."
            .into(),
        category: TemplateCategory::CodeAssistant,
        files: vec![
            TemplateFile {
                path: "prompts/system.md".into(),
                description: "System prompt for the code reviewer".into(),
                content: indoc(
                    "You are an expert code reviewer. When given a diff:\n\
                     1. Check for bugs, security issues, and performance problems.\n\
                     2. Suggest improvements with specific line references.\n\
                     3. Rate severity: critical, warning, or suggestion.\n\
                     4. Be constructive — explain why, not just what.",
                ),
            },
            TemplateFile {
                path: "prompts/review_format.md".into(),
                description: "Output format template for reviews".into(),
                content: indoc(
                    "## Review Summary\n\n\
                     **Overall:** {{verdict}}\n\n\
                     ## Findings\n\n\
                     {{#each findings}}\n\
                     - **{{severity}}** ({{file}}:{{line}}): {{message}}\n\
                     {{/each}}",
                ),
            },
            TemplateFile {
                path: "config/session.json".into(),
                description: "Session configuration for code review".into(),
                content: indoc(
                    "{\n  \"max_turns\": 10,\n  \"timeout_ms\": 60000,\n  \
                     \"temperature_milli\": 200,\n  \
                     \"structured_output\": true\n}",
                ),
            },
            TemplateFile {
                path: "config/provider.json".into(),
                description: "Provider binding for code review".into(),
                content: indoc(
                    "{\n  \"provider_family\": \"anthropic\",\n  \
                     \"model_id\": \"claude-sonnet-4-20250514\",\n  \
                     \"operation\": \"generate\",\n  \
                     \"required_capabilities\": [\"structured_output\"]\n}",
                ),
            },
            TemplateFile {
                path: "evals/review_quality.json".into(),
                description: "Eval suite for code review quality".into(),
                content: indoc(
                    "{\n  \"suite\": \"review_quality\",\n  \
                     \"evaluator\": \"rubric\",\n  \
                     \"dimensions\": [\n    {\n      \
                     \"name\": \"bug_detection\",\n      \"weight\": 0.4,\n      \
                     \"criteria\": \"task_success_rate\"\n    },\n    {\n      \
                     \"name\": \"suggestion_quality\",\n      \"weight\": 0.3,\n      \
                     \"criteria\": \"policy_pass_rate\"\n    },\n    {\n      \
                     \"name\": \"response_time\",\n      \"weight\": 0.3,\n      \
                     \"criteria\": \"latency_p50_ms\"\n    }\n  ]\n}",
                ),
            },
        ],
    }
}

fn data_analyst_template() -> Template {
    Template {
        id: "data-analyst".into(),
        name: "Data Analyst".into(),
        description: "Data processing agent with tool definitions for SQL queries, \
                      chart generation, and data summarization."
            .into(),
        category: TemplateCategory::DataPipeline,
        files: vec![
            TemplateFile {
                path: "prompts/system.md".into(),
                description: "System prompt for the data analyst".into(),
                content: indoc(
                    "You are a data analyst assistant. You can:\n\
                     1. Write and execute SQL queries using the sql.execute tool.\n\
                     2. Generate charts using the chart.create tool.\n\
                     3. Summarize datasets and identify trends.\n\n\
                     Always validate queries before execution. Never modify data \
                     without explicit confirmation.",
                ),
            },
            TemplateFile {
                path: "config/tools.json".into(),
                description: "Tool definitions for data operations".into(),
                content: indoc(
                    "{\n  \"tools\": [\n    {\n      \"name\": \"sql.execute\",\n      \
                     \"description\": \"Execute a read-only SQL query\",\n      \
                     \"permissions\": [\"db.read\"]\n    },\n    {\n      \
                     \"name\": \"chart.create\",\n      \
                     \"description\": \"Generate a chart from query results\",\n      \
                     \"permissions\": [\"fs.write\"]\n    },\n    {\n      \
                     \"name\": \"data.summarize\",\n      \
                     \"description\": \"Compute summary statistics\",\n      \
                     \"permissions\": []\n    }\n  ]\n}",
                ),
            },
            TemplateFile {
                path: "config/session.json".into(),
                description: "Session configuration for data analysis".into(),
                content: indoc(
                    "{\n  \"max_turns\": 30,\n  \"timeout_ms\": 120000,\n  \
                     \"temperature_milli\": 300\n}",
                ),
            },
            TemplateFile {
                path: "config/provider.json".into(),
                description: "Provider binding for data analysis".into(),
                content: indoc(
                    "{\n  \"provider_family\": \"openai\",\n  \
                     \"model_id\": \"gpt-4o\",\n  \
                     \"operation\": \"generate\",\n  \
                     \"required_capabilities\": [\"tool_use\"]\n}",
                ),
            },
            TemplateFile {
                path: "evals/data_accuracy.json".into(),
                description: "Eval suite for data analysis accuracy".into(),
                content: indoc(
                    "{\n  \"suite\": \"data_accuracy\",\n  \
                     \"evaluator\": \"contains\",\n  \
                     \"cases\": [\n    {\n      \
                     \"input\": \"How many rows in the users table?\",\n      \
                     \"expected\": \"SELECT COUNT\"\n    }\n  ]\n}",
                ),
            },
        ],
    }
}

/// Normalize indentation (pass-through — content is already formatted).
fn indoc(s: &str) -> String {
    s.to_owned()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_with_builtins_has_three_templates() {
        let reg = TemplateRegistry::with_builtins();
        assert_eq!(reg.len(), 3);
    }

    #[test]
    fn list_returns_sorted_summaries() {
        let reg = TemplateRegistry::with_builtins();
        let summaries = reg.list();
        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].id, "code-reviewer");
        assert_eq!(summaries[1].id, "data-analyst");
        assert_eq!(summaries[2].id, "simple-chatbot");
    }

    #[test]
    fn get_returns_full_template_with_files() {
        let reg = TemplateRegistry::with_builtins();
        let t = reg.get("simple-chatbot").unwrap();
        assert_eq!(t.name, "Simple Chatbot");
        assert_eq!(t.category, TemplateCategory::Chatbot);
        assert!(t.files.len() >= 3);
        assert!(t.files.iter().any(|f| f.path == "prompts/system.md"));
        assert!(t.files.iter().any(|f| f.path == "config/session.json"));
        assert!(t.files.iter().any(|f| f.path == "config/provider.json"));
        assert!(t.files.iter().any(|f| f.path.contains("evals/")));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let reg = TemplateRegistry::with_builtins();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn apply_creates_project_scoped_paths() {
        let reg = TemplateRegistry::with_builtins();
        let result = reg.apply("simple-chatbot", "my_project").unwrap();
        assert_eq!(result.template_id, "simple-chatbot");
        assert_eq!(result.project_id, "my_project");
        assert!(result
            .files_created
            .iter()
            .all(|f| f.starts_with("projects/my_project/")));
        assert!(result
            .files_created
            .iter()
            .any(|f| f.ends_with("prompts/system.md")));
    }

    #[test]
    fn apply_nonexistent_returns_none() {
        let reg = TemplateRegistry::with_builtins();
        assert!(reg.apply("nonexistent", "proj").is_none());
    }

    #[test]
    fn code_reviewer_template_structure() {
        let reg = TemplateRegistry::with_builtins();
        let t = reg.get("code-reviewer").unwrap();
        assert_eq!(t.category, TemplateCategory::CodeAssistant);
        assert!(t.files.len() >= 4);
        assert!(t.files.iter().any(|f| f.path == "prompts/review_format.md"));
        assert!(t.files.iter().any(|f| f.path == "evals/review_quality.json"));
    }

    #[test]
    fn data_analyst_template_has_tools() {
        let reg = TemplateRegistry::with_builtins();
        let t = reg.get("data-analyst").unwrap();
        assert_eq!(t.category, TemplateCategory::DataPipeline);
        assert!(t.files.iter().any(|f| f.path == "config/tools.json"));
        let tools_file = t.files.iter().find(|f| f.path == "config/tools.json").unwrap();
        assert!(tools_file.content.contains("sql.execute"));
        assert!(tools_file.content.contains("chart.create"));
        assert!(tools_file.content.contains("data.summarize"));
    }

    #[test]
    fn register_custom_template() {
        let mut reg = TemplateRegistry::new();
        assert!(reg.is_empty());

        reg.register(Template {
            id: "custom".into(),
            name: "Custom".into(),
            description: "A custom template".into(),
            category: TemplateCategory::CustomerSupport,
            files: vec![TemplateFile {
                path: "prompts/system.md".into(),
                description: "System prompt".into(),
                content: "Hello world".into(),
            }],
        });

        assert_eq!(reg.len(), 1);
        let t = reg.get("custom").unwrap();
        assert_eq!(t.category, TemplateCategory::CustomerSupport);
    }

    #[test]
    fn summary_has_correct_file_count() {
        let reg = TemplateRegistry::with_builtins();
        let summaries = reg.list();
        for s in &summaries {
            let full = reg.get(&s.id).unwrap();
            assert_eq!(s.file_count, full.files.len());
        }
    }

    #[test]
    fn all_templates_have_system_prompt() {
        let reg = TemplateRegistry::with_builtins();
        for t in reg.list() {
            let full = reg.get(&t.id).unwrap();
            assert!(
                full.files.iter().any(|f| f.path.contains("prompts/system")),
                "template {} missing system prompt",
                t.id
            );
        }
    }

    #[test]
    fn all_templates_have_eval_suite() {
        let reg = TemplateRegistry::with_builtins();
        for t in reg.list() {
            let full = reg.get(&t.id).unwrap();
            assert!(
                full.files.iter().any(|f| f.path.contains("evals/")),
                "template {} missing eval suite",
                t.id
            );
        }
    }

    #[test]
    fn all_templates_have_provider_config() {
        let reg = TemplateRegistry::with_builtins();
        for t in reg.list() {
            let full = reg.get(&t.id).unwrap();
            assert!(
                full.files.iter().any(|f| f.path.contains("config/")),
                "template {} missing config",
                t.id
            );
        }
    }
}
