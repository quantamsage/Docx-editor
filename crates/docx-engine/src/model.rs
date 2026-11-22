//! The read model: a typed, serializable view of a document's content.
//!
//! The model is derived from the XML; it is what the frontend displays.
//! Content the engine does not interpret appears as `Other` items so the
//! interface can show that something is there. Such content is always kept
//! in the document itself, whether or not it appears here.

use serde::{Deserialize, Serialize};

/// The whole document as seen by the editor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentModel {
    pub blocks: Vec<Block>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<Section>,
    pub styles: Vec<Style>,
    /// Header and footer parts referenced by the document.
    pub headers: Vec<String>,
    pub footers: Vec<String>,
    pub stats: Stats,
    /// Non-fatal problems found while reading (e.g. a damaged styles part).
    pub warnings: Vec<String>,
    /// List definitions from the numbering part, for drawing list markers.
    pub lists: Vec<crate::numbering::ListDefinition>,
}

/// Counts shown to the user after opening a document.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    pub paragraphs: usize,
    pub tables: usize,
    pub images: usize,
    pub hyperlinks: usize,
    /// Elements kept in the document but not interpreted by the editor.
    pub preserved_elements: usize,
    pub words: usize,
}

/// A body-level item.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
    /// Kept as-is (e.g. content controls, custom XML, alternate content).
    Other {
        kind: String,
        text: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Paragraph {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent: Option<Indent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<Spacing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numbering: Option<Numbering>,
    /// This paragraph ends a section (it carries its own `sectPr`).
    #[serde(skip_serializing_if = "is_false")]
    pub section_break: bool,
    pub inlines: Vec<Inline>,
}

impl Paragraph {
    /// The paragraph's visible text: runs and hyperlinks, with tabs as tab
    /// characters and line breaks as newlines. Preserved content contributes
    /// its text.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for inline in &self.inlines {
            match inline {
                Inline::Run(run) => run.push_text(&mut out),
                Inline::Hyperlink { runs, .. } => runs.iter().for_each(|r| r.push_text(&mut out)),
                Inline::Other { text, .. } => out.push_str(text),
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justify,
}

impl Alignment {
    /// Parses a `w:jc` value (transitional and strict spellings).
    pub fn from_ooxml(value: &str) -> Option<Self> {
        match value {
            "left" | "start" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" | "end" => Some(Self::Right),
            "both" | "distribute" => Some(Self::Justify),
            _ => None,
        }
    }

    /// The `w:jc` value written for this alignment.
    pub fn to_ooxml(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "both",
        }
    }
}

/// Indentation in twentieths of a point (twips).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Indent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<i32>,
    /// Positive for a first-line indent, negative for a hanging indent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_line: Option<i32>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Spacing in twips; `line` is in 240ths of a line when `line_rule` is
/// `auto` (240 = single), otherwise twips.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Spacing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_rule: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Numbering {
    pub num_id: String,
    pub level: u8,
}

/// Content inside a paragraph.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    Run(Run),
    Hyperlink {
        /// External URL, if the link points outside the document.
        url: Option<String>,
        /// Bookmark name for links within the document.
        anchor: Option<String>,
        runs: Vec<Run>,
    },
    /// Kept as-is (fields, tracked changes, content controls, …). `text` is
    /// the visible text it contains, if any.
    Other {
        kind: String,
        text: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Run {
    pub props: RunProps,
    pub content: Vec<RunContent>,
}

impl Run {
    fn push_text(&self, out: &mut String) {
        for item in &self.content {
            match item {
                RunContent::Text { text } => out.push_str(text),
                RunContent::Tab => out.push('\t'),
                RunContent::LineBreak => out.push('\n'),
                _ => {}
            }
        }
    }
}

/// Character formatting set directly on a run (not inherited from styles).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RunProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size in half-points (22 = 11 pt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    /// Hex RGB without `#`, e.g. `FF0000`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Highlight colour name, e.g. `yellow`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunContent {
    Text { text: String },
    Tab,
    LineBreak,
    PageBreak,
    ColumnBreak,
    Image(Image),
    Other { kind: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Image {
    /// Relationship id of the picture.
    pub rel_id: String,
    /// Part name of the image, e.g. `word/media/image1.png`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// Size in EMUs (914400 per inch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_emu: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_emu: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Floating (anchored) rather than inline with the text.
    #[serde(skip_serializing_if = "is_false")]
    pub floating: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Table {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    pub rows: Vec<TableRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TableCell {
    /// Number of grid columns spanned (`gridSpan`).
    pub span: u32,
    /// Vertical merge: `restart` begins a merged cell, `continue` extends it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_merge: Option<String>,
    pub blocks: Vec<Block>,
}

/// Page layout of the final (or only) section, in twips.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Section {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_height: Option<u32>,
    #[serde(skip_serializing_if = "is_false")]
    pub landscape: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_top: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_right: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_bottom: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_left: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_header: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_footer: Option<i32>,
    #[serde(skip_serializing_if = "is_false")]
    pub title_page: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Style {
    pub id: String,
    pub name: String,
    /// `paragraph`, `character`, `table` or `numbering`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub is_default: bool,
}
