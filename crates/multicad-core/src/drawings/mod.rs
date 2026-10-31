//! Authored drawing pages. Model views are separate linked content providers;
//! creating paper never creates geometry or illustrative model projections.
mod annotations;
pub mod dxf;
mod guides;
mod layout;
pub mod pdf;
pub mod render;
mod scale;
mod source;
pub mod svg;
mod title_block;
mod transactions;
mod views;
pub use annotations::*;
pub use guides::*;
pub use layout::*;
pub use scale::*;
pub use source::*;
pub use title_block::{
    CELL_MM, FRAME_MM, LABEL_MM, LOGO_PNG, PITCH, TITLE_MM, VALUE_MM, logo_idat, logo_size,
};
pub use transactions::*;
pub use views::*;
