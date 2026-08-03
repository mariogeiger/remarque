#[cfg(feature = "takeover")]
pub mod display;
#[cfg(feature = "takeover")]
pub mod document_requests;
#[cfg(feature = "takeover")]
mod draw_toolbar;
#[cfg(feature = "takeover")]
mod draw_viewport_indicators;
#[cfg(any(feature = "takeover", test))]
mod edge_page_swipe;
#[cfg(target_os = "linux")]
pub mod filter_touch_sequences;
#[cfg(target_os = "linux")]
pub mod input;
#[cfg(feature = "takeover")]
pub mod notebook;
#[cfg(any(feature = "takeover", test))]
mod page_coordinates;
#[cfg(feature = "takeover")]
mod pdfium;
#[cfg(feature = "takeover")]
mod quit_label;
#[cfg(feature = "takeover")]
pub mod screen_stream;
#[cfg(any(feature = "takeover", test))]
mod screen_stream_protocol;
pub mod toolbar;
pub use remarque_core::bgra_image;
pub use remarque_core::color;
pub use remarque_core::erase_strokes;
pub use remarque_core::fineliner;
pub use remarque_core::render_fineliner;
pub use remarque_core::stroke;
pub use remarque_core::view_transform;
