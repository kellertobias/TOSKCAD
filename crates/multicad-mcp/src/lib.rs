//! Local MCP control of MultiCAD over the same typed Session, solver and
//! native model APIs the desktop UI uses. No built-in AI model or chat.
pub mod error;
pub mod headless;
pub mod host;
#[cfg(unix)]
pub mod local_socket;
mod native_worker;
pub mod server;
pub mod tools;

pub use error::ToolError;
pub use headless::{Headless, Storage, Workspace};
pub use host::{Host, Job, Scope, Target};
pub use server::{Server, run_stdio};
