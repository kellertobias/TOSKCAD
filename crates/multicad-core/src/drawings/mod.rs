//! Authored drawing pages. Model views are separate linked content providers;
//! creating paper never creates geometry or illustrative model projections.
mod annotations;
mod layout;
mod source;
pub mod svg;
mod transactions;
mod views;
pub use annotations::*;
pub use layout::*;
pub use source::*;
pub use transactions::*;
pub use views::*;
