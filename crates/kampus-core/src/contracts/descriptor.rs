use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Canonical data classes. Schools map their own tiers onto these in
/// `policy.yaml` (`tier_mapping`); descriptors always use the canonical names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    Public,
    Internal,
    Sensitive,
    Confidential,
}

impl DataClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Sensitive => "sensitive",
            Self::Confidential => "confidential",
        }
    }
}

impl fmt::Display for DataClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical roles derived from verified identity; never from tool arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Student,
    Faculty,
    Staff,
    Guest,
    /// Local administration only; grants no data access by itself.
    Admin,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Student => "student",
            Self::Faculty => "faculty",
            Self::Staff => "staff",
            Self::Guest => "guest",
            Self::Admin => "admin",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Policy operation, derived from the descriptor, never from call arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Read,
    /// A read-only proposal that a separate confirmation tool can commit.
    Propose,
    /// The destructive confirmation of a proposal.
    Confirm,
}

/// Contents of a module's `module.yaml` (version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleDescriptor {
    pub version: u32,
    pub id: String,
    pub tools: Vec<ToolDescriptor>,
}

/// One task-level tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDescriptor {
    pub name: String,
    pub title: String,
    pub description: String,
    pub data_class: DataClass,
    /// Roles that may ever call the tool; policy rules narrow this further.
    pub roles: BTreeSet<Role>,
    /// True only for the destructive confirmation step of a proposal.
    pub write: bool,
    /// For a write tool: the read-only proposal tool it confirms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirms: Option<String>,
    /// OAuth scopes the caller's token must carry.
    #[serde(default)]
    pub scopes: BTreeSet<String>,
    #[serde(rename = "readOnlyHint")]
    pub read_only_hint: bool,
    #[serde(rename = "destructiveHint")]
    pub destructive_hint: bool,
    /// JSON Schema (object) for the tool arguments.
    pub input_schema: Value,
    /// JSON Schema (object) for `data` in the success envelope. The published
    /// MCP `outputSchema` is [`crate::contracts::envelope_output_schema`] of this.
    pub output_schema: Value,
}

/// Why a descriptor was refused at registration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DescriptorError {
    #[error("module {module}: unsupported module.yaml version {version}")]
    Version { module: String, version: u32 },
    #[error("module {module}: invalid module.yaml: {message}")]
    Parse { module: String, message: String },
    #[error("tool {tool}: {message}")]
    Tool { tool: String, message: &'static str },
}

impl ModuleDescriptor {
    /// Parse and validate a `module.yaml` document.
    pub fn from_yaml(module: &str, yaml: &str) -> Result<Self, DescriptorError> {
        let descriptor: Self =
            serde_saphyr::from_str(yaml).map_err(|e| DescriptorError::Parse {
                module: module.to_owned(),
                message: e.to_string(),
            })?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Enforce the annotation and confirmation rules for every tool.
    pub fn validate(&self) -> Result<(), DescriptorError> {
        if self.version != 1 {
            return Err(DescriptorError::Version {
                module: self.id.clone(),
                version: self.version,
            });
        }
        for tool in &self.tools {
            let fail = |message| {
                Err(DescriptorError::Tool {
                    tool: tool.name.clone(),
                    message,
                })
            };
            if !is_tool_name(&tool.name) {
                return fail("name must be 1-64 chars of [a-z0-9_]");
            }
            if tool.roles.is_empty() {
                return fail("roles must not be empty");
            }
            if tool.data_class == DataClass::Confidential {
                return fail("confidential data has no modules");
            }
            if !is_object_schema(&tool.input_schema) || !is_object_schema(&tool.output_schema) {
                return fail("input_schema and output_schema must be object schemas");
            }
            if tool.write {
                if tool.read_only_hint || !tool.destructive_hint {
                    return fail("write tools need readOnlyHint=false and destructiveHint=true");
                }
                let Some(target) = tool.confirms.as_deref() else {
                    return fail("write tools must name the proposal tool they confirm");
                };
                let proposal_is_read = self
                    .tools
                    .iter()
                    .any(|t| t.name == target && !t.write && t.name != tool.name);
                if !proposal_is_read {
                    return fail("confirms must name a read-only tool in the same module");
                }
            } else {
                if !tool.read_only_hint || tool.destructive_hint {
                    return fail("read tools need readOnlyHint=true and destructiveHint=false");
                }
                if tool.confirms.is_some() {
                    return fail("only write tools may declare confirms");
                }
            }
        }
        Ok(())
    }

    /// The operation a tool performs, derived from the descriptor only.
    pub fn operation(&self, tool: &ToolDescriptor) -> Operation {
        if tool.write {
            Operation::Confirm
        } else if self
            .tools
            .iter()
            .any(|t| t.confirms.as_deref() == Some(tool.name.as_str()))
        {
            Operation::Propose
        } else {
            Operation::Read
        }
    }
}

fn is_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_object_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
}
