//! Authored drawing pages. Model views are separate linked content providers;
//! creating paper never creates geometry or illustrative model projections.
mod layout;
mod source;
mod transactions;
pub use layout::*;
pub use source::*;
pub use transactions::*;
