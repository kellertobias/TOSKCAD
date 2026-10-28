//! Reusable assembly definitions and directed, explicitly driven rigid joints.
mod parameters;
mod prepare;
mod resolve;
mod source;
pub use parameters::*;
pub use prepare::*;
pub use resolve::*;
pub use source::*;
pub const JOINT_TYPE: &str = "model.assembly_joint";
pub const ASSEMBLY_TYPE: &str = "model.assembly_component";
pub const NAMED_POSE_TYPE: &str = "model.assembly_pose";
pub const JOINT_CAPABILITY: &str = "model.assembly_joint.1";
pub const ASSEMBLY_CAPABILITY: &str = "model.assembly_component.1";
pub const NAMED_POSE_CAPABILITY: &str = "model.assembly_pose.1";
