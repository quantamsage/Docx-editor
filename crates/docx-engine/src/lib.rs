//! Lossless DOCX document engine.
//!
//! The engine reads a `.docx` file into a [`Document`] without discarding
//! anything it does not understand, exposes a typed read model of the
//! content ([`DocumentModel`]), applies editing operations by patching the
//! original XML, and writes the document back:
//!
//! * Parts that were not edited are written back byte-for-byte (same order,
//!   compression and timestamps).
//! * In the edited main document part, elements that were not touched keep
//!   their exact original markup; new properties are inserted in schema
//!   order next to any unknown siblings.
//!
//! This crate has no knowledge of HTTP or async runtimes.

mod blank;
pub mod check;
mod document;
pub mod edit;
pub mod error;
pub mod format;
pub mod history;
pub mod media;
pub mod model;
pub mod numbering;
pub mod opc;
pub mod package;
mod props;
mod read;
pub mod table;
pub mod xml;

pub use document::{Document, Snapshot};
pub use edit::{CellRef, Container, Edit, HeaderFooterKind, InlineRef, Piece, Splice};
pub use error::{DocxError, Result};
pub use model::DocumentModel;
pub use package::{Compression, Limits, Package, Part};
pub use read::count_words;
