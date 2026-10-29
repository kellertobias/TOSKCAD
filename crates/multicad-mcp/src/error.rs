//! Structured tool failures. Every error names a stable machine code, the
//! involved canonical identities and, where relevant, the live revision.
use multicad_core::{
    commands::{CommandError, ErrorCode},
    document::{DocumentError, EntityId},
    parameters::ParameterError,
};
use multicad_solver::{EditError, InputError};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub involved: Vec<EntityId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<u64>,
}
impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            involved: vec![],
            current_revision: None,
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }
    pub fn not_found(id: EntityId, what: &str) -> Self {
        Self::new("not_found", format!("{what} {id} does not exist")).involving(id)
    }
    pub fn cancelled() -> Self {
        Self::new(
            "cancelled",
            "The request was cancelled; the document is unchanged",
        )
    }
    pub fn involving(mut self, id: EntityId) -> Self {
        self.involved.push(id);
        self
    }
    pub fn at_revision(mut self, revision: u64) -> Self {
        self.current_revision = Some(revision);
        self
    }
}
impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ToolError {}
impl From<CommandError> for ToolError {
    fn from(error: CommandError) -> Self {
        let code = match error.code {
            ErrorCode::StaleRevision => "stale_revision",
            ErrorCode::ForeignPreview => "stale_preview",
            ErrorCode::InvalidRequest => "invalid_request",
            ErrorCode::InvalidDocument => "invalid_document",
            ErrorCode::InvalidParameter => "invalid_parameter",
            ErrorCode::NoHistory => "no_history",
            ErrorCode::RevisionLimit => "revision_limit",
        };
        Self {
            code,
            message: error.message,
            involved: error.involved,
            current_revision: None,
        }
    }
}
impl From<InputError> for ToolError {
    fn from(error: InputError) -> Self {
        Self {
            code: "invalid_sketch",
            message: error.message,
            involved: error.source.into_iter().collect(),
            current_revision: None,
        }
    }
}
impl From<EditError> for ToolError {
    fn from(error: EditError) -> Self {
        match error {
            EditError::Input(e) => e.into(),
            EditError::Command(e) => e.into(),
        }
    }
}
impl From<DocumentError> for ToolError {
    fn from(error: DocumentError) -> Self {
        Self::new("invalid_document", error.to_string())
    }
}
impl From<ParameterError> for ToolError {
    fn from(error: ParameterError) -> Self {
        Self {
            code: "invalid_parameter",
            message: error.to_string(),
            involved: error.entity.into_iter().collect(),
            current_revision: None,
        }
    }
}
impl From<multicad_model::ModelError> for ToolError {
    fn from(error: multicad_model::ModelError) -> Self {
        Self {
            code: "model_failed",
            message: error.message,
            involved: error.entity.into_iter().collect(),
            current_revision: None,
        }
    }
}
