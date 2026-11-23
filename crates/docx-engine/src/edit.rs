//! Editing operations, as sent by the editor.
//!
//! The central operation is [`Splice`]: it replaces the content of one or
//! more consecutive paragraphs with a list of [`Piece`]s. A piece either
//! keeps an existing inline element (a run, hyperlink, image, field, …)
//! unchanged, adds text formatted like an existing run, adds a picture or a
//! page break, or starts a new paragraph. Typing, deleting, Enter, joining
//! paragraphs, deleting across paragraphs, pasting and inserting pictures
//! are all splices, so one carefully tested operation covers every text
//! edit.
//!
//! Paragraphs are addressed by their index among the blocks of a
//! [`Container`]: the document body (the default), a header or footer, or a
//! table cell in either.

use serde::Deserialize;

use crate::format::{CellFormat, ParagraphFormat, RunFormat, SectionFormat};
use crate::model::Alignment;

/// Identifies an inline of the current model: `blocks[block].inlines[inline]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub struct InlineRef {
    pub block: usize,
    pub inline: usize,
}

/// A table cell: `blocks[table].rows[row].cells[cell]` of the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellRef {
    pub table: usize,
    pub row: usize,
    pub cell: usize,
}

/// Where the blocks an edit addresses live.
///
/// * `part`: a header or footer part of the document (e.g.
///   `word/header1.xml`), or `None` for the main document body.
/// * `cell`: a table cell inside that part; block indices then count the
///   cell's paragraphs and tables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    #[serde(default)]
    pub part: Option<String>,
    #[serde(default)]
    pub cell: Option<CellRef>,
}

impl Container {
    /// The main document body.
    pub fn body() -> Self {
        Self::default()
    }

    /// A table cell of the main document body.
    pub fn cell(table: usize, row: usize, cell: usize) -> Self {
        Self { part: None, cell: Some(CellRef { table, row, cell }) }
    }

    /// A header or footer part.
    pub fn part(name: &str) -> Self {
        Self { part: Some(name.to_string()), cell: None }
    }
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
    /// A page break (`<w:br w:type="page"/>`) in a run of its own.
    PageBreak,
    /// An inline picture. `image` is the id returned by
    /// [`crate::Document::stage_image`]; the picture's part, relationship
    /// and content type are added to the document with the edit. Sizes are
    /// in EMUs (914400 per inch).
    Image {
        image: String,
        width_emu: i64,
        height_emu: i64,
        #[serde(default)]
        description: Option<String>,
    },
}

/// Replaces paragraphs `first..=last` of `container` with `content`.
///
/// * Every inline of those paragraphs that is not kept is removed.
/// * Kept inlines must appear in document order.
/// * Markers that are not content (bookmarks, comment ranges, permission
///   ranges, proofing marks) are kept in their order; new content is placed
///   where the first removed inline was, or else before the next kept one.
/// * The first resulting paragraph keeps the first paragraph's element and
///   properties; each `Break` starts a paragraph with the same properties.
///   A section break carried by the last paragraph ends up on the last
///   resulting paragraph. Splices that would remove a section break are
///   refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Splice {
    #[serde(default)]
    pub container: Container,
    pub first: usize,
    pub last: usize,
    pub content: Vec<Piece>,
}

/// Header or footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeaderFooterKind {
    Header,
    Footer,
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
        #[serde(default)]
        container: Container,
        first: usize,
        last: usize,
        format: ParagraphFormat,
    },
    /// Inserts an empty table with `rows` × `columns` cells so that it
    /// becomes block `at` of the container (a body or a header/footer, not
    /// a cell).
    InsertTable {
        #[serde(default)]
        container: Container,
        at: usize,
        rows: usize,
        columns: usize,
    },
    /// Inserts an empty row above (or, with `after`, below) row `row`.
    InsertRow {
        #[serde(default)]
        container: Container,
        table: usize,
        row: usize,
        #[serde(default)]
        after: bool,
    },
    /// Deletes row `row` (a table keeps at least one row).
    DeleteRow {
        #[serde(default)]
        container: Container,
        table: usize,
        row: usize,
    },
    /// Inserts a column left (or, with `after`, right) of the cell
    /// `rows[row].cells[cell]`.
    InsertColumn {
        #[serde(default)]
        container: Container,
        table: usize,
        row: usize,
        cell: usize,
        #[serde(default)]
        after: bool,
    },
    /// Deletes the grid column where the cell `rows[row].cells[cell]`
    /// starts (a table keeps at least one column).
    DeleteColumn {
        #[serde(default)]
        container: Container,
        table: usize,
        row: usize,
        cell: usize,
    },
    /// Shows (single lines) or hides all borders of a table.
    SetTableBorders {
        #[serde(default)]
        container: Container,
        table: usize,
        borders: bool,
    },
    /// Changes cell properties (vertical alignment, shading) of the cell
    /// `rows[row].cells[cell]`.
    FormatCell {
        #[serde(default)]
        container: Container,
        table: usize,
        row: usize,
        cell: usize,
        format: CellFormat,
    },
    /// Sets a picture's display size (EMUs), keeping everything else.
    ResizeImage {
        #[serde(default)]
        container: Container,
        block: usize,
        inline: usize,
        width_emu: i64,
        height_emu: i64,
    },
    /// Changes the page layout of every section (paper size, orientation,
    /// margins, page number start).
    FormatSection {
        format: SectionFormat,
    },
    /// Makes sure the document has a header (or footer) on every page,
    /// creating an empty one where a section has none.
    AddHeaderFooter {
        kind: HeaderFooterKind,
    },
    /// Adds a paragraph with page numbers ("1", or "Page 1 of 3" with
    /// `with_total`) to the header or footer, creating it if needed.
    InsertPageNumbers {
        kind: HeaderFooterKind,
        #[serde(default = "default_page_number_alignment")]
        alignment: Alignment,
        #[serde(default)]
        with_total: bool,
    },
}

fn default_page_number_alignment() -> Alignment {
    Alignment::Center
}
