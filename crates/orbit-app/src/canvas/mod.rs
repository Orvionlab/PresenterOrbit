//! Shared canvas, object inspector and editing history for slides, themes, props, masks and stage layouts.
//! Document adapters own loading and persistence; the interaction surface stays the same.

mod builds;
mod catalog;
mod content_document;
mod document;
mod document_inspector;
mod documents_ui;
mod editing;
mod geometry;
mod guides;
mod inspector;
mod nodes;
mod object_fields;
mod presets;
mod prop_inspector;
mod raster;
mod rich_panel;
mod saves;
mod sidebar;
mod surface;
mod toolbar;
mod view;
pub use content_document::DocumentKind;
pub use view::{CanvasEvent, SlideCanvas};
