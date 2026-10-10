use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use jsonschema::error::ValidationErrorKind;
use jsonschema::paths::{Location, LocationSegment};
use serde_json::Value;

use super::{
    DescriptorError, Module, ModuleDescriptor, Operation, ToolDescriptor, ToolError,
    envelope_output_schema,
};

/// Registration failure; the server refuses to start.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    Descriptor(#[from] DescriptorError),
    #[error("duplicate module id {0}")]
    DuplicateModule(String),
    #[error("duplicate tool name {0}")]
    DuplicateTool(String),
    #[error("tool {tool}: invalid JSON Schema in {which}")]
    Schema { tool: String, which: &'static str },
    #[error("module {module}: {message}")]
    Module { module: String, message: String },
}

/// A registered tool with its compiled schemas.
pub struct RegisteredTool {
    pub descriptor: ToolDescriptor,
    pub operation: Operation,
    /// The published MCP `outputSchema` (the envelope union).
    pub output_schema: Value,
    pub module: Arc<dyn Module>,
    input_validator: jsonschema::Validator,
    output_validator: jsonschema::Validator,
}

impl fmt::Debug for RegisteredTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredTool")
            .field("name", &self.descriptor.name)
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}

impl RegisteredTool {
    /// Validate call arguments against `input_schema`.
    ///
    /// The error message names what is wrong using only schema-sourced text
    /// (declared property names and limits); argument values and unknown
    /// keys are never echoed.
    pub fn validate_input(&self, args: &Value) -> Result<(), ToolError> {
        if self.input_validator.is_valid(args) {
            return Ok(());
        }
        let declared = &self.descriptor.input_schema["properties"];
        let mut problems: Vec<String> = Vec::new();
        for error in self.input_validator.iter_errors(args) {
            let problem = describe(error.kind(), error.instance_path(), declared);
            if !problems.contains(&problem) {
                problems.push(problem);
            }
            if problems.len() == MAX_PROBLEMS {
                break;
            }
        }
        let message = if problems.is_empty() {
            "Arguments do not match the tool schema.".to_owned()
        } else {
            format!("Invalid arguments: {}", problems.join(" "))
        };
        Err(ToolError::invalid_arguments(message))
    }

    /// Whether a projected envelope matches the published `outputSchema`.
    pub fn output_is_valid(&self, envelope: &Value) -> bool {
        self.output_validator.is_valid(envelope)
    }
}

/// At most this many distinct problems are reported per call.
const MAX_PROBLEMS: usize = 3;

/// One sentence about a schema violation, built from schema text only.
fn describe(kind: &ValidationErrorKind, path: &Location, declared: &Value) -> String {
    let names = |props: &Value| -> Vec<String> {
        props
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };
    // Name the top-level property only when the schema declares it.
    let field = match path.segments().next() {
        Some(LocationSegment::Property(name)) => declared
            .as_object()
            .and_then(|m| m.get_key_value(name.as_ref()))
            .map(|(k, _)| k.clone()),
        _ => None,
    };
    let subject = field.unwrap_or_else(|| "an argument".to_owned());
    match kind {
        ValidationErrorKind::Required { property } => match property.as_str() {
            Some(p) if declared.get(p).is_some() => format!("{p} is required."),
            _ => "A required argument is missing.".to_owned(),
        },
        ValidationErrorKind::AnyOf { context } | ValidationErrorKind::OneOfNotValid { context } => {
            let mut alternatives: Vec<&str> = context
                .iter()
                .flatten()
                .filter_map(|e| match e.kind() {
                    ValidationErrorKind::Required { property } => property.as_str(),
                    _ => None,
                })
                .filter(|p| declared.get(*p).is_some())
                .collect();
            alternatives.dedup();
            if alternatives.len() > 1 && context.iter().all(|c| c.len() == 1) {
                format!("Provide at least one of: {}.", alternatives.join(", "))
            } else {
                "The arguments do not match any accepted combination.".to_owned()
            }
        }
        ValidationErrorKind::AdditionalProperties { .. } => {
            let allowed = names(declared);
            if allowed.is_empty() {
                "Unknown arguments are not allowed.".to_owned()
            } else {
                format!("Unknown argument; allowed: {}.", allowed.join(", "))
            }
        }
        ValidationErrorKind::Type { .. } if path.as_str().is_empty() => {
            "Arguments must be a JSON object.".to_owned()
        }
        ValidationErrorKind::Type { .. } => format!("{subject} has the wrong type."),
        ValidationErrorKind::Format { format } => format!("{subject} must be a valid {format}."),
        ValidationErrorKind::MinLength { limit } => {
            format!("{subject} must be at least {limit} characters.")
        }
        ValidationErrorKind::MaxLength { limit } => {
            format!("{subject} must be at most {limit} characters.")
        }
        ValidationErrorKind::Minimum { limit } => format!("{subject} must be at least {limit}."),
        ValidationErrorKind::Maximum { limit } => format!("{subject} must be at most {limit}."),
        ValidationErrorKind::Pattern { .. } => format!("{subject} has an invalid format."),
        ValidationErrorKind::Enum { .. } | ValidationErrorKind::Constant { .. } => {
            format!("{subject} has an unsupported value.")
        }
        _ => format!("{subject} does not match the tool schema."),
    }
}

/// All tools the server exposes, in registration order. Tools are listed for
/// every caller; policy decides at call time.
#[derive(Debug, Default)]
pub struct ToolRegistry {
    tools: Vec<RegisteredTool>,
    by_name: BTreeMap<String, usize>,
    module_ids: Vec<String>,
}

impl ToolRegistry {
    pub fn new(modules: Vec<Arc<dyn Module>>) -> Result<Self, RegistryError> {
        let mut registry = Self::default();
        for module in modules {
            registry.register(&module)?;
        }
        Ok(registry)
    }

    /// Validate and add one module. Unsafe descriptors are rejected here.
    pub fn register(&mut self, module: &Arc<dyn Module>) -> Result<(), RegistryError> {
        let descriptor: &ModuleDescriptor = module.descriptor();
        descriptor.validate()?;
        if self.module_ids.contains(&descriptor.id) {
            return Err(RegistryError::DuplicateModule(descriptor.id.clone()));
        }
        let mut staged = Vec::with_capacity(descriptor.tools.len());
        for tool in &descriptor.tools {
            if self.by_name.contains_key(&tool.name)
                || staged
                    .iter()
                    .any(|t: &RegisteredTool| t.descriptor.name == tool.name)
            {
                return Err(RegistryError::DuplicateTool(tool.name.clone()));
            }
            let schema_error = |which| RegistryError::Schema {
                tool: tool.name.clone(),
                which,
            };
            let output_schema = envelope_output_schema(&tool.output_schema);
            let input_validator = jsonschema::validator_for(&tool.input_schema)
                .map_err(|_| schema_error("input_schema"))?;
            let output_validator = jsonschema::validator_for(&output_schema)
                .map_err(|_| schema_error("output_schema"))?;
            staged.push(RegisteredTool {
                operation: descriptor.operation(tool),
                descriptor: tool.clone(),
                output_schema,
                module: Arc::clone(module),
                input_validator,
                output_validator,
            });
        }
        self.module_ids.push(descriptor.id.clone());
        for tool in staged {
            self.by_name
                .insert(tool.descriptor.name.clone(), self.tools.len());
            self.tools.push(tool);
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.by_name.get(name).map(|&i| &self.tools[i])
    }

    pub fn tools(&self) -> impl Iterator<Item = &RegisteredTool> {
        self.tools.iter()
    }

    pub fn tool_names(&self) -> impl Iterator<Item = &str> {
        self.tools.iter().map(|t| t.descriptor.name.as_str())
    }

    pub fn module_ids(&self) -> &[String] {
        &self.module_ids
    }
}
