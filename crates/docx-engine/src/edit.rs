//! Editing operations, as sent by the editor.
//!
//! The central operation is [`Splice`]: it replaces the content of one or
//! more consecutive paragraphs with a list of [`Piece`]s. A piece either
//! keeps an existing inline element (a run, hyperlink, image, field, …)
//! unchanged, adds text formatted like an existing run, or starts a new
//! paragraph. Typing, deleting, Enter, joining paragraphs, deleting across
//! paragraphs and pasting are all splices, so one carefully tested
//! operation covers every text edit.

use serde::Deserialize;

use crate::format::{ParagraphFormat, RunFormat};
use crate::model::Alignment;

/// Identifies an inline of the current model: `blocks[block].inlines[inline]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub struct InlineRef {
    pub block: usize,
    pub inline: usize,
}

/// One piece of new paragraph content.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Piece {
    /// Keep an existing inline exactly as it is.
    Keep { block: usize, inline: usize },
    /// New text, formatted like the run `like` (or unformatted). `\t` is a
    /// tab and `\n` a line break.
    Text {
        text: String,
        #[serde(default)]
        like: Option<InlineRef>,
        /// Formatting changes applied on top of `like`'s formatting.
        #[serde(default)]
        format: Option<RunFormat>,
    },
    /// Ends the current paragraph and starts a new one.
    Break,
}

/// Replaces paragraphs `first..=last` with `content`.
///
/// * Every inline of those paragraphs that is not kept is removed.
/// * Kept inlines must appear in document order.
/// * Markers that are not content (bookmarks, comment ranges, permission
///   ranges, proofing marks) are kept in their order; new text is placed
///   where the first removed inline was, or else before the next kept one.
/// * The first resulting paragraph keeps the first paragraph's element and
///   properties; each `Break` starts a paragraph with the same properties.
///   A section break carried by the last paragraph ends up on the last
///   resulting paragraph. Splices that would remove a section break are
///   refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Splice {
    pub first: usize,
    pub last: usize,
    pub content: Vec<Piece>,
}

/// An editing operation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    Splice(Splice),
    InsertParagraph {
        at: usize,
        #[serde(default)]
        text: String,
        #[serde(default)]
        style_id: Option<String>,
    },
    DeleteBlock {
        block: usize,
    },
    SetParagraphStyle {
        block: usize,
        style_id: Option<String>,
    },
    SetParagraphAlignment {
        block: usize,
        alignment: Option<Alignment>,
    },
    /// Changes the formatting of paragraphs `first..=last`.
    FormatParagraphs {
        first: usize,
        last: usize,
        format: ParagraphFormat,
    },
}
