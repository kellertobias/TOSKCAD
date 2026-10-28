//! Non-driving inspection intent and revision-bound measurements. Native
//! evaluation owns numeric values; canonical pins contain references only.
mod source;
mod values;
pub use source::*;
pub use values::*;
