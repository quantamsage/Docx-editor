//! Tables: inserting tables, editing cell text through cell containers,
//! inserting and deleting rows and columns (keeping spans, merges and
//! widths consistent), borders and cell properties.

mod common;

use common::*;
use docx_engine::format::{CellFormat, ParagraphFormat, VerticalAlign};
use docx_engine::model::{Alignment, Block, Paragraph, Table};
use docx_engine::{Container, Document, DocumentModel, Edit, InlineRef, Piece, Splice};

fn open() -> Document {
    Document::open(&representative_docx()).unwrap()
}

fn word_sample() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/word-sample.docx")).unwrap()
}

fn table(model: &DocumentModel, index: usize) -> &Table {
    match &model.blocks[index] {
        Block::Table(t) => t,
        other => panic!("block {index} is not a table: {other:?}"),
    }
}

fn cell_text(t: &Table, row: usize, cell: usize) -> String {
    t.rows[row].cells[cell]
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.plain_text(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn spans(t: &Table) -> Vec<Vec<(u32, Option<String>)>> {
    t.rows.iter().map(|r| r.cells.iter().map(|c| (c.span, c.vertical_merge.clone())).collect()).collect()
}

fn round_trip(doc: &Document) -> DocumentModel {
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    Document::open(&saved).unwrap().model()
}

fn kinds(model: &DocumentModel) -> Vec<&'static str> {
    model
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(_) => "p",
            Block::Table(_) => "tbl",
            Block::Other { .. } => "other",
        })
        .collect()
}

#[test]
fn inserting_a_table_in_a_new_document() {
    let mut doc = Document::new_blank();
    doc.apply(&[Edit::InsertTable { container: Container::body(), at: 1, rows: 2, columns: 3 }]).unwrap();
    let model = round_trip(&doc);
    assert_eq!(kinds(&model), ["p", "tbl", "p"], "a paragraph follows the table so the user can type after it");
    let t = table(&model, 1);
    assert_eq!(t.rows.len(), 2);
    assert!(t.rows.iter().all(|r| r.cells.len() == 3));
    assert_eq!(t.grid.iter().sum::<u32>(), 11906 - 2 * 1440, "the table spans the text width");
    assert_eq!((t.borders, t.style_id.as_deref()), (Some(true), Some("TableGrid")));

    let xml = part_text(&doc.save().unwrap(), "word/document.xml");
    assert!(xml.contains(r#"<w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#), "{xml}");
    assert!(xml.contains(r#"<w:tc><w:tcPr><w:tcW w:w="3009" w:type="dxa"/></w:tcPr><w:p/></w:tc>"#), "{xml}");
}

#[test]
fn tables_never_touch_each_other() {
    let mut doc = open();
    // Right after the existing table (block 4), before "First item".
    doc.apply(&[Edit::InsertTable { container: Container::body(), at: 5, rows: 1, columns: 1 }]).unwrap();
    let model = round_trip(&doc);
    assert_eq!(&kinds(&model)[3..8], ["p", "tbl", "p", "tbl", "p"]);
    assert_eq!(model.blocks.len(), 13);

    for (rows, columns) in [(0, 2), (2, 0), (2, 64), (501, 1)] {
        let err = doc.apply(&[Edit::InsertTable { container: Container::body(), at: 0, rows, columns }]).unwrap_err();
        assert!(err.to_string().contains("tables can have"), "{err}");
    }
    let err = doc
        .apply(&[Edit::InsertTable { container: Container::cell(4, 0, 0), at: 0, rows: 1, columns: 1 }])
        .unwrap_err();
    assert!(err.to_string().contains("inside a table cell"), "{err}");
}

#[test]
fn cell_text_is_edited_with_splices_in_the_cell() {
    let bytes = representative_docx();
    let mut doc = Document::open(&bytes).unwrap();
    let cell = Container::cell(4, 1, 1); // "B"
    let like = Some(InlineRef { block: 0, inline: 0 });
    doc.splice(&Splice {
        container: cell.clone(),
        first: 0,
        last: 0,
        content: vec![
            Piece::Text { text: "Beta".into(), like, format: None },
            Piece::Break,
            Piece::Text { text: "second line".into(), like, format: None },
        ],
    })
    .unwrap();
    doc.apply(&[Edit::FormatParagraphs {
        container: cell,
        first: 0,
        last: 1,
        format: ParagraphFormat { alignment: Some(Some(Alignment::Center)), ..Default::default() },
    }])
    .unwrap();
    let model = round_trip(&doc);
    let t = table(&model, 4);
    assert_eq!(cell_text(t, 1, 1), "Beta|second line");
    let Block::Paragraph(p) = &t.rows[1].cells[1].blocks[1] else { panic!() };
    assert_eq!(p.alignment, Some(Alignment::Center));
    assert_eq!(cell_text(t, 1, 0), "A", "other cells are untouched");
    assert_eq!(changed_parts(&bytes, &doc.save().unwrap()), ["word/document.xml"]);

    // Addresses outside the table are refused.
    for (container, expected) in [
        (Container::cell(3, 0, 0), "block 3 is not a table"),
        (Container::cell(4, 9, 0), "no cell 0 in row 9"),
        (Container::cell(4, 0, 1), "no cell 1 in row 0"),
    ] {
        let err = doc.splice(&Splice { container, first: 0, last: 0, content: vec![] }).unwrap_err();
        assert!(err.to_string().contains(expected), "{err}");
    }
}

#[test]
fn inserted_rows_copy_the_structure_and_stay_inside_merges() {
    let mut doc = open();
    // Rows: [span 2], [A restart, B], [continue, C].
    doc.apply(&[Edit::InsertRow { container: Container::body(), table: 4, row: 1, after: true }]).unwrap();
    let model = round_trip(&doc);
    let t = table(&model, 4);
    assert_eq!(
        spans(t),
        vec![
            vec![(2, None)],
            vec![(1, Some("restart".into())), (1, None)],
            vec![(1, Some("continue".into())), (1, None)],
            vec![(1, Some("continue".into())), (1, None)],
        ],
        "the new row is inside the vertical merge"
    );
    assert_eq!(cell_text(t, 2, 1), "");
    assert_eq!(cell_text(t, 3, 1), "C");

    // Above the first row: one cell spanning both columns.
    doc.apply(&[Edit::InsertRow { container: Container::body(), table: 4, row: 0, after: false }]).unwrap();
    let t = &table(&round_trip(&doc), 4).clone();
    assert_eq!(spans(t)[0], vec![(2, None)]);
    assert_eq!(cell_text(t, 1, 0), "Merged header");
}

#[test]
fn deleting_rows_keeps_merges_and_one_row() {
    let mut doc = open();
    doc.apply(&[Edit::DeleteRow { container: Container::body(), table: 4, row: 1 }]).unwrap();
    let model = round_trip(&doc);
    let t = table(&model, 4);
    assert_eq!(spans(t), vec![vec![(2, None)], vec![(1, Some("restart".into())), (1, None)]]);
    assert_eq!(cell_text(t, 1, 1), "C");

    doc.apply(&[Edit::DeleteRow { container: Container::body(), table: 4, row: 0 }]).unwrap();
    let err = doc.apply(&[Edit::DeleteRow { container: Container::body(), table: 4, row: 0 }]).unwrap_err();
    assert!(err.to_string().contains("at least one row"), "{err}");
}

#[test]
fn inserting_columns_keeps_spans_and_total_width() {
    let mut doc = open();
    // Right of "B": a new last column; the merged header cell is not widened.
    doc.apply(&[Edit::InsertColumn { container: Container::body(), table: 4, row: 1, cell: 1, after: true }]).unwrap();
    let model = round_trip(&doc);
    let t = table(&model, 4);
    assert_eq!(t.grid.len(), 3);
    assert_eq!(t.grid.iter().sum::<u32>(), 9016, "the table keeps its width");
    assert_eq!(spans(t).iter().map(Vec::len).collect::<Vec<_>>(), [2, 3, 3]);
    assert_eq!(cell_text(t, 1, 1), "B");

    // Left of "B" (grid column 1): inside the merged header, which widens.
    doc.apply(&[Edit::InsertColumn { container: Container::body(), table: 4, row: 1, cell: 1, after: false }]).unwrap();
    let model = round_trip(&doc);
    let t = table(&model, 4);
    assert_eq!(t.grid.len(), 4);
    assert_eq!(spans(t)[0][0], (3, None));
    assert_eq!(cell_text(t, 1, 2), "B");
    let xml = part_text(&doc.save().unwrap(), "word/document.xml");
    // The header cell's fixed width follows the three columns it covers.
    let width: u32 = t.grid[..3].iter().sum();
    assert!(xml.contains(&format!(r#"<w:tcW w:w="{width}" w:type="dxa"/><w:gridSpan w:val="3"/>"#)), "{xml}");
}

#[test]
fn deleting_columns_narrows_spans_and_removes_cells() {
    let mut doc = open();
    doc.apply(&[Edit::DeleteColumn { container: Container::body(), table: 4, row: 1, cell: 0 }]).unwrap();
    let model = round_trip(&doc);
    let t = table(&model, 4);
    assert_eq!(t.grid, [9016]);
    assert_eq!(spans(t), vec![vec![(1, None)], vec![(1, None)], vec![(1, None)]]);
    assert_eq!(
        (cell_text(t, 0, 0), cell_text(t, 1, 0), cell_text(t, 2, 0)),
        ("Merged header".into(), "B".into(), "C".into())
    );
    assert!(!part_text(&doc.save().unwrap(), "word/document.xml").contains("gridSpan"));

    let err = doc.apply(&[Edit::DeleteColumn { container: Container::body(), table: 4, row: 0, cell: 0 }]).unwrap_err();
    assert!(err.to_string().contains("at least one column"), "{err}");
}

#[test]
fn borders_and_cell_properties() {
    let mut doc = open();
    assert_eq!(table(&doc.model(), 4).borders, None, "the style decides");
    doc.apply(&[Edit::SetTableBorders { container: Container::body(), table: 4, borders: false }]).unwrap();
    assert_eq!(table(&round_trip(&doc), 4).borders, Some(false));
    let xml = part_text(&doc.save().unwrap(), "word/document.xml");
    assert!(xml.contains(r#"<w:tblW w:w="0" w:type="auto"/><w:tblBorders><w:top w:val="nil"/>"#), "{xml}");
    doc.apply(&[Edit::SetTableBorders { container: Container::body(), table: 4, borders: true }]).unwrap();
    assert_eq!(table(&round_trip(&doc), 4).borders, Some(true));

    let format = CellFormat { vertical_align: Some(Some(VerticalAlign::Center)), shading: Some(Some("d9e2f3".into())) };
    doc.apply(&[Edit::FormatCell { container: Container::body(), table: 4, row: 0, cell: 0, format }]).unwrap();
    let model = round_trip(&doc);
    let cell = &table(&model, 4).rows[0].cells[0];
    assert_eq!((cell.vertical_align, cell.shading.as_deref()), (Some(VerticalAlign::Center), Some("D9E2F3")));
    let xml = part_text(&doc.save().unwrap(), "word/document.xml");
    assert!(xml.contains(r#"<w:tcPr><w:tcW w:w="9016" w:type="dxa"/><w:gridSpan w:val="2"/><w:shd w:val="clear" w:color="auto" w:fill="D9E2F3"/><w:vAlign w:val="center"/></w:tcPr>"#), "{xml}");

    let clear = CellFormat { vertical_align: Some(None), shading: Some(None) };
    doc.apply(&[Edit::FormatCell { container: Container::body(), table: 4, row: 0, cell: 0, format: clear }]).unwrap();
    let cell = table(&round_trip(&doc), 4).rows[0].cells[0].clone();
    assert_eq!((cell.vertical_align, cell.shading), (None, None));

    let bad = CellFormat { shading: Some(Some("blue".into())), ..Default::default() };
    assert!(
        doc.apply(&[Edit::FormatCell { container: Container::body(), table: 4, row: 0, cell: 0, format: bad }])
            .is_err()
    );
}

#[test]
fn tables_with_shifted_rows_are_not_restructured() {
    let xml = DOCUMENT_XML.replace(
        "<w:tr><w:tc><w:tcPr><w:vMerge w:val=\"restart\"/>",
        "<w:tr><w:trPr><w:gridBefore w:val=\"1\"/></w:trPr><w:tc><w:tcPr><w:vMerge w:val=\"restart\"/>",
    );
    let mut doc = Document::open(&docx_with(&[("word/document.xml", Some(&xml))])).unwrap();
    let err = doc
        .apply(&[Edit::InsertColumn { container: Container::body(), table: 4, row: 0, cell: 0, after: true }])
        .unwrap_err();
    assert!(err.to_string().contains("can't be changed"), "{err}");
    assert!(!doc.is_modified());
}

#[test]
fn word_tables_keep_everything_else() {
    let bytes = word_sample();
    let mut doc = Document::open(&bytes).unwrap();
    let index = doc.model().blocks.iter().position(|b| matches!(b, Block::Table(_))).unwrap();
    doc.apply(&[
        Edit::InsertRow { container: Container::body(), table: index, row: 2, after: true },
        Edit::InsertColumn { container: Container::body(), table: index, row: 0, cell: 2, after: true },
        Edit::FormatCell {
            container: Container::body(),
            table: index,
            row: 0,
            cell: 0,
            format: CellFormat { shading: Some(Some("FFF2CC".into())), ..Default::default() },
        },
    ])
    .unwrap();
    let saved = doc.save().unwrap();
    assert_eq!(changed_parts(&bytes, &saved), ["word/document.xml"]);
    let model = round_trip(&doc);
    let t = table(&model, index);
    assert_eq!((t.rows.len(), t.grid.len()), (4, 4));
    assert_eq!(t.grid.iter().sum::<u32>(), 9026);
    assert_eq!(cell_text(t, 0, 0), "Region");
    // Blocks before and after the table are unchanged.
    let original = Document::open(&bytes).unwrap().model();
    let texts = |m: &DocumentModel| -> Vec<String> {
        m.blocks
            .iter()
            .filter_map(|b| if let Block::Paragraph(p) = b { Some(Paragraph::plain_text(p)) } else { None })
            .collect()
    };
    assert_eq!(texts(&model), texts(&original));
}

#[test]
fn table_edits_deserialize_from_json() {
    let edits: Vec<Edit> = serde_json::from_str(
        r#"[
            {"op": "insert_table", "at": 2, "rows": 2, "columns": 3},
            {"op": "insert_row", "table": 4, "row": 0, "after": true},
            {"op": "delete_column", "table": 4, "row": 1, "cell": 0},
            {"op": "set_table_borders", "table": 4, "borders": false},
            {"op": "format_cell", "table": 4, "row": 0, "cell": 0, "format": {"vertical_align": "bottom", "shading": null}},
            {"op": "splice", "container": {"cell": {"table": 4, "row": 1, "cell": 1}}, "first": 0, "last": 0, "content": []}
        ]"#,
    )
    .unwrap();
    assert_eq!(edits.len(), 6);
    match &edits[5] {
        Edit::Splice(s) => assert_eq!(s.container, Container::cell(4, 1, 1)),
        other => panic!("{other:?}"),
    }
    assert!(
        serde_json::from_str::<Vec<Edit>>(
            r#"[{"op": "splice", "container": {"cel": 1}, "first": 0, "last": 0, "content": []}]"#
        )
        .is_err()
    );
}
