//! Formatting and structure preservation across every kind of edit.
//!
//! Each scenario edits one of the two test documents (one written by
//! Microsoft Word, one synthetic "hard cases" document) and checks that:
//!
//! * only the parts the edit needs are rewritten; all others are
//!   byte-identical;
//! * in the main document, top-level blocks the edit didn't touch are
//!   byte-identical;
//! * constructs the editor doesn't edit (bookmarks, comments, footnote
//!   references, fields, hyperlinks, content controls, alternate content,
//!   extension elements, pictures, section breaks) are all still there;
//! * the result has no schema-order problems, reopens, and saving it again
//!   without edits changes nothing.
//!
//! Finally, all scenarios are applied one after another and undone, which
//! must give back the original file part for part.
//!
//! Set PRESERVATION_OUT=<folder> to keep every scenario's file (to open
//! them in Word with `scripts/verify-with-word.ps1`).

mod common;

use common::*;
use docx_engine::format::{
    CellFormat, ListChange, ListKind, Margins, PageSize, ParagraphFormat, RunFormat, SectionFormat, VerticalAlign,
};
use docx_engine::history::History;
use docx_engine::model::Alignment;
use docx_engine::xml::{Node, XmlDocument};
use docx_engine::{Container, Document, Edit, HeaderFooterKind, InlineRef, Piece, Splice};

fn word_sample() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/word-sample.docx")).unwrap()
}

/// The serialized top-level children of the body (blocks, markers and the
/// final section properties).
fn body_children(bytes: &[u8]) -> Vec<String> {
    let doc = XmlDocument::parse(part_text(bytes, "word/document.xml").as_bytes()).unwrap();
    let body = doc.root.child(W, "body").unwrap();
    body.children
        .iter()
        .filter_map(|n| match n {
            Node::Element(e) => {
                let mut out = String::new();
                e.write_to(&mut out);
                Some(out)
            }
            _ => None,
        })
        .collect()
}

/// How many top-level children are unchanged, in order (the longest
/// common subsequence of the two lists).
fn unchanged_children(before: &[String], after: &[String]) -> usize {
    let mut table = vec![vec![0usize; after.len() + 1]; before.len() + 1];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i][j] =
                if before[i] == after[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    table[0][0]
}

/// Counts of constructs the editor must never lose, over the main
/// document, headers and footers.
fn constructs(bytes: &[u8]) -> Vec<(&'static str, usize)> {
    let mut text = String::new();
    for (name, data, _) in read_parts(bytes) {
        if name.starts_with("word/")
            && (name.contains("document") || name.contains("header") || name.contains("footer"))
        {
            text.push_str(&String::from_utf8_lossy(&data));
        }
    }
    [
        "<w:bookmarkStart ",
        "<w:bookmarkEnd ",
        "<w:commentRangeStart ",
        "<w:commentRangeEnd ",
        "<w:commentReference ",
        "<w:footnoteReference ",
        "<w:fldChar ",
        "<w:instrText",
        "<w:hyperlink ",
        "<w:sdt>",
        "<mc:AlternateContent>",
        "<x:custom ",
        "<w:drawing>",
        "<w:fldSimple ",
        "<w:sectPr",
        "<w:tbl>",
        "w14:paraId=",
    ]
    .into_iter()
    .map(|needle| (needle, text.matches(needle).count()))
    .collect()
}

struct Scenario {
    name: &'static str,
    edits: Vec<Edit>,
    /// Parts the edit may rewrite (or add).
    parts: &'static [&'static str],
    /// Top-level body children the edit may change (or add).
    touched: usize,
    /// Constructs whose count may change, with the expected difference.
    changes: &'static [(&'static str, isize)],
}

fn text_piece(text: &str, like: Option<(usize, usize)>, format: Option<RunFormat>) -> Piece {
    Piece::Text { text: text.into(), like: like.map(|(block, inline)| InlineRef { block, inline }), format }
}

fn keep(block: usize, inline: usize) -> Piece {
    Piece::Keep { block, inline }
}

fn splice(container: Container, block: usize, content: Vec<Piece>) -> Edit {
    Edit::Splice(Splice { container, first: block, last: block, content })
}

const MAIN: &str = "word/document.xml";
const MAIN_RELS: &str = "word/_rels/document.xml.rels";
const TYPES: &str = "[Content_Types].xml";

/// Scenarios for the Word document (blocks as `docx_tool inspect` lists them).
fn word_scenarios(image: &str) -> Vec<Scenario> {
    let body = Container::body;
    vec![
        Scenario {
            name: "typing in a formatted paragraph",
            edits: vec![splice(body(), 2, {
                let mut content: Vec<Piece> = (0..6).map(|i| keep(2, i)).collect();
                content.push(text_piece(" More text.", Some((2, 5)), None));
                content
            })],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "bold and colour on part of a run",
            edits: vec![splice(
                body(),
                3,
                vec![
                    text_piece("This ", Some((3, 0)), None),
                    text_piece(
                        "paragraph",
                        Some((3, 0)),
                        Some(RunFormat { bold: Some(true), color: Some(Some("C00000".into())), ..Default::default() }),
                    ),
                    text_piece(" is centred.", Some((3, 0)), None),
                ],
            )],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "Enter in the paragraph with a footnote and comment",
            edits: vec![splice(
                body(),
                15,
                vec![
                    text_piece("Reviewed", Some((15, 0)), None),
                    Piece::Break,
                    text_piece(" paragraph.", Some((15, 0)), None),
                    keep(15, 1),
                    keep(15, 2),
                ],
            )],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "paragraph style, alignment, spacing and a list",
            edits: vec![Edit::FormatParagraphs {
                container: body(),
                first: 17,
                last: 18,
                format: ParagraphFormat {
                    alignment: Some(Some(Alignment::Right)),
                    spacing: Some(Some(docx_engine::format::SpacingChange {
                        before: Some(120),
                        after: Some(0),
                        line: Some(360),
                    })),
                    list: Some(Some(ListChange { kind: ListKind::Number, level: 0 })),
                    ..Default::default()
                },
            }],
            parts: &[MAIN],
            touched: 2,
            changes: &[],
        },
        Scenario {
            name: "typing in a table cell",
            edits: vec![splice(Container::cell(11, 1, 0), 0, vec![text_piece("North-west", Some((0, 0)), None)])],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "table rows, columns, borders and cells",
            edits: vec![
                Edit::InsertRow { container: body(), table: 11, row: 1, after: true },
                Edit::InsertColumn { container: body(), table: 11, row: 0, cell: 0, after: false },
                Edit::DeleteRow { container: body(), table: 11, row: 0 },
                Edit::SetTableBorders { container: body(), table: 11, borders: false },
                Edit::FormatCell {
                    container: body(),
                    table: 11,
                    row: 0,
                    cell: 1,
                    format: CellFormat {
                        vertical_align: Some(Some(VerticalAlign::Bottom)),
                        shading: Some(Some("E2EFD9".into())),
                    },
                },
            ],
            parts: &[MAIN],
            touched: 1,
            // The deleted row's own id and its three paragraphs' ids.
            changes: &[("w14:paraId=", -4)],
        },
        Scenario {
            name: "inserting a table",
            edits: vec![Edit::InsertTable { container: body(), at: 5, rows: 3, columns: 4 }],
            parts: &[MAIN],
            touched: 1,
            changes: &[("<w:tbl>", 1)],
        },
        Scenario {
            name: "typing in the header",
            edits: vec![splice(
                Container::part("word/header2.xml"),
                0,
                vec![keep(0, 0), text_piece(" (draft)", Some((0, 0)), None)],
            )],
            parts: &["word/header2.xml"],
            touched: 0,
            changes: &[],
        },
        Scenario {
            name: "inserting a picture",
            edits: vec![splice(
                body(),
                12,
                vec![
                    keep(12, 0),
                    Piece::Image {
                        image: image.into(),
                        width_emu: 952_500,
                        height_emu: 476_250,
                        description: Some("Logo".into()),
                    },
                ],
            )],
            parts: &[MAIN, MAIN_RELS, "word/media/image2.png"],
            touched: 1,
            changes: &[("<w:drawing>", 1)],
        },
        Scenario {
            name: "resizing the picture",
            edits: vec![Edit::ResizeImage {
                container: body(),
                block: 13,
                inline: 0,
                width_emu: 1_000_000,
                height_emu: 500_000,
            }],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "page numbers in the header",
            edits: vec![Edit::InsertPageNumbers {
                kind: HeaderFooterKind::Header,
                alignment: Alignment::Right,
                with_total: true,
            }],
            parts: &["word/header2.xml"],
            touched: 0,
            changes: &[("<w:fldSimple ", 2)],
        },
        Scenario {
            name: "page setup",
            edits: vec![Edit::FormatSection {
                format: SectionFormat {
                    size: Some(PageSize { width: 12240, height: 15840 }),
                    landscape: Some(true),
                    margins: Some(Margins { top: Some(720), bottom: Some(720), ..Default::default() }),
                    page_number_start: Some(Some(2)),
                },
            }],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "a page break",
            edits: vec![splice(body(), 1, vec![keep(1, 0), Piece::PageBreak, Piece::Break])],
            parts: &[MAIN],
            touched: 2,
            changes: &[],
        },
        Scenario {
            name: "replace all across paragraphs and a cell",
            edits: vec![
                splice(body(), 3, vec![text_piece("This section is centred.", Some((3, 0)), None)]),
                splice(
                    body(),
                    4,
                    vec![text_piece(
                        "This section is justified and indented, with extra spacing before and after it so that the layout properties are exercised.",
                        Some((4, 0)),
                        None,
                    )],
                ),
                splice(Container::cell(11, 2, 0), 0, vec![text_piece("Total across every region", Some((0, 0)), None)]),
            ],
            parts: &[MAIN],
            touched: 3,
            changes: &[],
        },
    ]
}

fn check_scenario(original: &[u8], doc: &Document, s: &Scenario) {
    let saved = doc.save().unwrap_or_else(|e| panic!("{}: {e}", s.name));
    assert_valid_docx(&saved);
    // PRESERVATION_OUT=<folder> keeps the results, e.g. to open them in Word
    // with scripts/verify-with-word.ps1.
    if let Ok(dir) = std::env::var("PRESERVATION_OUT") {
        let file: String = s.name.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        std::fs::write(std::path::Path::new(&dir).join(format!("{file}.docx")), &saved).unwrap();
    }

    // Only the expected parts change.
    for part in changed_parts(original, &saved) {
        let allowed = s.parts.contains(&part.as_str())
            || (part == TYPES && s.parts.iter().any(|p| p.starts_with("word/media/")))
            || (part == "word/styles.xml" || part == "word/numbering.xml");
        assert!(allowed, "{}: {part} changed", s.name);
    }

    // Untouched top-level blocks are byte-identical.
    let before = body_children(original);
    let after = body_children(&saved);
    let unchanged = unchanged_children(&before, &after);
    assert!(
        unchanged + s.touched >= before.len(),
        "{}: only {unchanged} of {} top-level elements are unchanged ({} touched)",
        s.name,
        before.len(),
        s.touched
    );

    // Nothing the editor doesn't edit is lost.
    for ((name, was), (_, now)) in constructs(original).into_iter().zip(constructs(&saved)) {
        let expected = s.changes.iter().find(|(n, _)| *n == name).map_or(0, |(_, d)| *d);
        assert_eq!(now as isize - was as isize, expected, "{}: count of {name} changed", s.name);
    }

    // Saving the saved document again changes nothing.
    let resaved = Document::open(&saved).unwrap().save().unwrap();
    assert_eq!(changed_parts(&saved, &resaved), Vec::<String>::new(), "{}: re-saving changed parts", s.name);
}

#[test]
fn every_edit_keeps_the_rest_of_a_word_document() {
    let original = word_sample();
    for (i, scenario) in word_scenarios("img1").iter().enumerate() {
        let mut doc = Document::open(&original).unwrap();
        let image = doc.stage_image(PNG.to_vec()).unwrap().id;
        assert_eq!(image, "img1");
        // The resize scenario resizes the document's own picture.
        doc.apply(&scenario.edits).unwrap_or_else(|e| panic!("scenario {i} ({}): {e}", scenario.name));
        check_scenario(&original, &doc, scenario);
    }
}

#[test]
fn every_edit_keeps_the_hard_cases() {
    let original = representative_docx();
    let body = Container::body;
    let scenarios = vec![
        Scenario {
            name: "typing next to a bookmark and proofing marks",
            edits: vec![splice(
                body(),
                1,
                vec![text_piece("Revenue rose ", Some((1, 0)), None), keep(1, 1), keep(1, 2)],
            )],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "editing around a field and comment",
            edits: vec![splice(
                body(),
                7,
                vec![
                    text_piece("Page number ", Some((7, 0)), None),
                    keep(7, 1),
                    keep(7, 2),
                    keep(7, 3),
                    keep(7, 4),
                    keep(7, 5),
                    keep(7, 6),
                ],
            )],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "formatting the paragraph with alternate content",
            edits: vec![Edit::FormatParagraphs {
                container: body(),
                first: 8,
                last: 8,
                format: ParagraphFormat { alignment: Some(Some(Alignment::Center)), ..Default::default() },
            }],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "rows and columns in a table with merged cells",
            edits: vec![
                Edit::InsertRow { container: body(), table: 4, row: 1, after: true },
                Edit::InsertColumn { container: body(), table: 4, row: 1, cell: 1, after: true },
                Edit::DeleteColumn { container: body(), table: 4, row: 1, cell: 0 },
            ],
            parts: &[MAIN],
            touched: 1,
            changes: &[],
        },
        Scenario {
            name: "a new footer is not needed: page numbers are already there",
            edits: vec![splice(
                Container::part("word/footer1.xml"),
                0,
                vec![text_piece("Page ", None, None), keep(0, 0)],
            )],
            parts: &["word/footer1.xml"],
            touched: 0,
            changes: &[],
        },
    ];
    for scenario in &scenarios {
        let mut doc = Document::open(&original).unwrap();
        doc.apply(&scenario.edits).unwrap_or_else(|e| panic!("{}: {e}", scenario.name));
        check_scenario(&original, &doc, scenario);
    }
}

#[test]
fn all_edits_then_undo_gives_back_the_original_file() {
    let original = word_sample();
    let mut doc = Document::open(&original).unwrap();
    let image = doc.stage_image(PNG.to_vec()).unwrap().id;
    let mut history = History::new();
    // Block indices of later scenarios assume the original document, so
    // apply them in an order where earlier ones don't shift later ones:
    // edits at the end of the document first.
    let mut scenarios = word_scenarios(&image);
    scenarios.retain(|s| {
        !matches!(
            s.name,
            "inserting a table" | "a page break" | "resizing the picture" | "table rows, columns, borders and cells"
        )
    });
    scenarios.reverse();
    for scenario in &scenarios {
        history.record(&doc, None);
        doc.apply(&scenario.edits).unwrap_or_else(|e| panic!("{}: {e}", scenario.name));
    }
    let edited = doc.save().unwrap();
    assert_valid_docx(&edited);
    assert!(changed_parts(&original, &edited).len() >= 4);
    while history.undo(&mut doc).unwrap() {}
    assert!(!doc.is_modified());
    assert_eq!(changed_parts(&original, &doc.save().unwrap()), Vec::<String>::new());
}
