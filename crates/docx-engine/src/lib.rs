//! Lossless DOCX document engine.
//!
//! The engine reads a `.docx` file into a [`Document`] without discarding
//! anything it does not understand, exposes a typed read model of the
//! content, applies editing operations by patching the original XML, and
//! writes the document back. Parts that were not edited are written back
//! byte-for-byte; within the edited main document part, elements that were
//! not touched keep their exact original markup.
//!
//! This crate has no knowledge of HTTP or async runtimes.

pub mod error;
pub mod package;

pub use error::{DocxError, Result};
pub use package::{Compression, Limits, Package, Part};
