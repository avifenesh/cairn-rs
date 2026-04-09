//! cairn-tools-derive: Proc macro for deriving the ToolHandler trait.
//!
//! Adopted from Cersei's `#[derive(Tool)]` (MIT, pacifio/cersei), adapted for
//! Cairn's `ToolHandler` trait which takes `&ProjectKey` + `serde_json::Value`.
//!
//! ## Usage
//!
//! ```ignore
//! use cairn_tools_derive::Tool;
//! use cairn_tools::builtins::{ToolHandler, ToolResult, ToolError, ToolContext};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize, schemars::JsonSchema)]
//! struct MyInput {
//!     query: String,
//!     limit: Option<u32>,
//! }
//!
//! #[derive(Tool)]
//! #[tool(name = "my_tool", description = "Does something useful")]
//! struct MyTool;
//!
//! #[async_trait::async_trait]
//! impl cairn_tools::builtins::ToolExecute for MyTool {
//!     type Input = MyInput;
//!     async fn execute_typed(
//!         &self, _project: &cairn_domain::ProjectKey, input: MyInput, _ctx: &ToolContext,
//!     ) -> Result<ToolResult, ToolError> {
//!         Ok(ToolResult::ok(serde_json::json!({ "result": input.query })))
//!     }
//! }
//! ```
//!
//! ## Attributes
//!
//! | Attribute       | Required | Default         | Values |
//! |-----------------|----------|-----------------|--------|
//! | `name`          | no       | struct lowercase| string |
//! | `description`   | no       | `""`            | string |
//! | `tier`          | no       | `registered`    | core, registered, deferred |
//! | `permission`    | no       | `none`          | none, read_only, write, execute, dangerous, forbidden |
//! | `category`      | no       | `custom`        | filesystem, shell, web, memory, orchestration, query, custom |

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(Tool, attributes(tool))]
pub fn derive_tool(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let mut tool_name = name.to_string().to_lowercase();
    let mut tool_description = String::new();
    let mut tool_tier = quote! { cairn_tools::builtins::ToolTier::Registered };
    let mut tool_permission = quote! { cairn_tools::builtins::PermissionLevel::None };
    let mut tool_category = quote! { cairn_tools::builtins::ToolCategory::Custom };

    for attr in &input.attrs {
        if !attr.path().is_ident("tool") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(ident) = meta.path.get_ident() {
                let key = ident.to_string();
                let value: syn::LitStr = meta.value()?.parse()?;
                let val = value.value();
                match key.as_str() {
                    "name" => tool_name = val,
                    "description" => tool_description = val,
                    "tier" => {
                        tool_tier = match val.as_str() {
                            "core" => quote! { cairn_tools::builtins::ToolTier::Core },
                            "registered" => quote! { cairn_tools::builtins::ToolTier::Registered },
                            "deferred" => quote! { cairn_tools::builtins::ToolTier::Deferred },
                            _ => quote! { cairn_tools::builtins::ToolTier::Registered },
                        };
                    }
                    "permission" => {
                        tool_permission = match val.as_str() {
                            "none" => quote! { cairn_tools::builtins::PermissionLevel::None },
                            "read_only" => {
                                quote! { cairn_tools::builtins::PermissionLevel::ReadOnly }
                            }
                            "write" => quote! { cairn_tools::builtins::PermissionLevel::Write },
                            "execute" => quote! { cairn_tools::builtins::PermissionLevel::Execute },
                            "dangerous" => {
                                quote! { cairn_tools::builtins::PermissionLevel::Dangerous }
                            }
                            "forbidden" => {
                                quote! { cairn_tools::builtins::PermissionLevel::Forbidden }
                            }
                            _ => quote! { cairn_tools::builtins::PermissionLevel::None },
                        };
                    }
                    "category" => {
                        tool_category = match val.as_str() {
                            "filesystem" => {
                                quote! { cairn_tools::builtins::ToolCategory::FileSystem }
                            }
                            "shell" => quote! { cairn_tools::builtins::ToolCategory::Shell },
                            "web" => quote! { cairn_tools::builtins::ToolCategory::Web },
                            "memory" => quote! { cairn_tools::builtins::ToolCategory::Memory },
                            "orchestration" => {
                                quote! { cairn_tools::builtins::ToolCategory::Orchestration }
                            }
                            "query" => quote! { cairn_tools::builtins::ToolCategory::Query },
                            _ => quote! { cairn_tools::builtins::ToolCategory::Custom },
                        };
                    }
                    _ => {}
                }
            }
            Ok(())
        });
    }

    let expanded = quote! {
        #[async_trait::async_trait]
        impl cairn_tools::builtins::ToolHandler for #name {
            fn name(&self) -> &str {
                #tool_name
            }

            fn tier(&self) -> cairn_tools::builtins::ToolTier {
                #tool_tier
            }

            fn description(&self) -> &str {
                #tool_description
            }

            fn permission_level(&self) -> cairn_tools::builtins::PermissionLevel {
                #tool_permission
            }

            fn category(&self) -> cairn_tools::builtins::ToolCategory {
                #tool_category
            }

            fn parameters_schema(&self) -> serde_json::Value {
                let schema = schemars::schema_for!(
                    <Self as cairn_tools::builtins::ToolExecute>::Input
                );
                serde_json::to_value(schema).unwrap_or(serde_json::json!({}))
            }

            async fn execute(
                &self,
                project: &cairn_domain::ProjectKey,
                args: serde_json::Value,
            ) -> Result<cairn_tools::builtins::ToolResult, cairn_tools::builtins::ToolError> {
                match serde_json::from_value::<<Self as cairn_tools::builtins::ToolExecute>::Input>(args) {
                    Ok(typed_input) => {
                        <Self as cairn_tools::builtins::ToolExecute>::execute_typed(
                            self, project, typed_input, &cairn_tools::builtins::ToolContext::default(),
                        ).await
                    }
                    Err(e) => Err(cairn_tools::builtins::ToolError::InvalidArgs {
                        field: "input".into(),
                        message: format!("Invalid input for '{}': {}", #tool_name, e),
                    }),
                }
            }
        }
    };

    TokenStream::from(expanded)
}
