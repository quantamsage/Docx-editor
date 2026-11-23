//! Character and paragraph formatting: run formats applied through splices,
//! paragraph formats, styles added on demand, and lists (reusing or
//! creating numbering definitions).

mod common;

use common::*;
use docx_engine::format::{IndentChange, ListChange, ListKind, ParagraphFormat, RunFormat, SpacingChange};
use docx_engine::model::{Alignment, Block, Inline, Paragraph};
use docx_engine::{Document, DocumentModel, Edit, InlineRef, Piece, Splice};

fn open() -> Document {
    Document::open(&representative_docx()).unwrap()
}

fn paragraph(model: &DocumentModel, index: usize) -> &Paragraph {
    match &model.blocks[index] {
        Block::Paragraph(p) => p,
        other => panic!("block {index} is not a paragraph: {other:?}"),
    }
}

fn run_props(p: &Paragraph, inline: usize) -> &docx_engine::model::RunProps {
    match &p.inlines[inline] {
        Inline::Run(r) => &r.props,
        other => panic!("inline {inline} is not a run: {other:?}"),
    }
}

fn keep(block: usize, inline: usize) -> Piece {
    Piece::Keep { block, inline }
}

fn formatted(text: &str, like: (usize, usize), format: RunFormat) -> Piece {
    Piece::Text { text: text.into(), like: Some(InlineRef { block: like.0, inline: like.1 }), format: Some(format) }
}

fn round_trip(doc: &Document) -> DocumentModel {
    Document::open(&doc.save().unwrap()).unwrap().model()
}

fn main_xml(doc: &Document) -> String {
    part_text(&doc.save().unwrap(), "word/document.xml")
}

#[test]
fn run_formatting_sets_properties_in_schema_order_and_keeps_the_rest() {
    let mut doc = open();
    // Block 1: "Revenue grew " + bold red "12%" + italic Georgia " this quarter".
    let all = RunFormat {
        bold: Some(true),
        underline: Some(true),
        font: Some(Some("Arial".into())),
        size: Some(Some(32)),
        color: Some(Some("1f4e79".into())),
        highlight: Some(Some("cyan".into())),
        ..RunFormat::default()
    };
    doc.splice(&Splice {
        container: Default::default(),
        first: 1,
        last: 1,
        content: vec![keep(1, 0), keep(1, 1), formatted(" this quarter", (1, 2), all)],
    })
    .unwrap();
    let model = round_trip(&doc);
    let p = paragraph(&model, 1);
    let props = run_props(p, 2);
    assert_eq!(
        (props.bold, props.italic, props.underline),
        (Some(true), Some(true), Some(true)),
        "italic is kept from the run"
    );
    assert_eq!(
        (props.font.as_deref(), props.size, props.color.as_deref(), props.highlight.as_deref()),
        (Some("Arial"), Some(32), Some("1F4E79"), Some("cyan"))
    );
    assert_eq!(p.plain_text(), "Revenue grew 12% this quarter");

    let xml = main_xml(&doc);
    assert!(
        xml.contains(r#"<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/><w:b/><w:i/><w:strike/><w:color w:val="1F4E79"/><w:sz w:val="32"/><w:szCs w:val="32"/><w:highlight w:val="cyan"/><w:u w:val="single"/></w:rPr>"#),
        "properties must follow the CT_RPr sequence:\n{xml}"
    );
}

#[test]
fn run_formatting_can_turn_off_and_remove_properties() {
    let mut doc = open();
    let off = RunFormat { bold: Some(false), color: Some(None), ..RunFormat::default() };
    doc.splice(&Splice {
        container: Default::default(),
        first: 1,
        last: 1,
        content: vec![keep(1, 0), formatted("12%", (1, 1), off), keep(1, 2)],
    })
    .unwrap();
    let model = round_trip(&doc);
    let props = run_props(paragraph(&model, 1), 1);
    assert_eq!((props.bold, props.color.as_deref()), (Some(false), None));
    assert!(main_xml(&doc).contains(r#"<w:rPr><w:b w:val="0"/></w:rPr><w:t>12%</w:t>"#));

    // Removing every property drops the empty rPr.
    let mut doc = open();
    let none = RunFormat { bold: Some(true), ..RunFormat::default() };
    doc.splice(&Splice {
        container: Default::default(),
        first: 0,
        last: 0,
        content: vec![formatted("Quarterly Report", (0, 0), none)],
    })
    .unwrap();
    assert!(main_xml(&doc).contains(r#"<w:r><w:rPr><w:b/></w:rPr><w:t>Quarterly Report</w:t></w:r>"#));
}

#[test]
fn invalid_run_formats_are_rejected() {
    for format in [
        RunFormat { color: Some(Some("red".into())), ..RunFormat::default() },
        RunFormat { size: Some(Some(1)), ..RunFormat::default() },
        RunFormat { font: Some(Some("x".repeat(40))), ..RunFormat::default() },
        RunFormat { font: Some(Some("Bad\"Font".into())), ..RunFormat::default() },
        RunFormat { highlight: Some(Some("purple".into())), ..RunFormat::default() },
    ] {
        let mut doc = open();
        let err = doc
            .splice(&Splice {
                container: Default::default(),
                first: 0,
                last: 0,
                content: vec![formatted("x", (0, 0), format.clone())],
            })
            .unwrap_err();
        assert!(matches!(err, docx_engine::DocxError::InvalidEdit(_)), "{format:?}");
        assert!(!doc.is_modified());
    }
}

#[test]
fn paragraph_alignment_indentation_and_spacing() {
    let mut doc = open();
    let format = ParagraphFormat {
        alignment: Some(Some(Alignment::Center)),
        indent: Some(Some(IndentChange { left: Some(720), right: None, first_line: Some(-360) })),
        spacing: Some(Some(SpacingChange { before: Some(240), after: Some(120), line: Some(360) })),
        ..ParagraphFormat::default()
    };
    doc.format_paragraphs(1, 3, &format).unwrap();
    let model = round_trip(&doc);
    for block in 1..=3 {
        let p = paragraph(&model, block);
        assert_eq!(p.alignment, Some(Alignment::Center), "block {block}");
        let ind = p.indent.as_ref().unwrap();
        assert_eq!((ind.left, ind.first_line), (Some(720), Some(-360)));
        let sp = p.spacing.as_ref().unwrap();
        assert_eq!(
            (sp.before, sp.after, sp.line, sp.line_rule.as_deref()),
            (Some(240), Some(120), Some(360), Some("auto"))
        );
    }
    assert_eq!(paragraph(&model, 0).alignment, None, "other paragraphs are untouched");
    assert!(main_xml(&doc).contains(r#"<w:pPr><w:spacing w:before="240" w:after="120" w:line="360" w:lineRule="auto"/><w:ind w:left="720" w:hanging="360"/><w:jc w:val="center"/></w:pPr>"#));

    // The justified paragraph's existing spacing is changed, not duplicated;
    // removing alignment and indentation leaves spacing alone.
    let last = doc.block_count() - 1;
    doc.format_paragraphs(
        last,
        last,
        &ParagraphFormat { alignment: Some(None), indent: Some(None), ..ParagraphFormat::default() },
    )
    .unwrap();
    let model = round_trip(&doc);
    let p = paragraph(&model, last);
    assert_eq!((p.alignment, p.indent.is_none()), (None, true));
    assert_eq!(p.spacing.as_ref().unwrap().before, Some(120));
}

#[test]
fn missing_heading_styles_are_added_to_the_style_sheet() {
    let mut doc = open();
    assert!(!doc.model().styles.iter().any(|s| s.id == "Heading2"));
    doc.format_paragraphs(
        2,
        2,
        &ParagraphFormat { style_id: Some(Some("Heading2".into())), ..ParagraphFormat::default() },
    )
    .unwrap();
    let saved = doc.save().unwrap();
    let reopened = Document::open(&saved).unwrap().model();
    assert_eq!(paragraph(&reopened, 2).style_id.as_deref(), Some("Heading2"));
    let heading = reopened.styles.iter().find(|s| s.id == "Heading2").expect("style added");
    assert_eq!((heading.name.as_str(), heading.kind.as_str()), ("heading 2", "paragraph"));
    // Existing styles are untouched; the definition is appended.
    let styles = part_text(&saved, "word/styles.xml");
    assert!(styles.starts_with(&STYLES_XML[..STYLES_XML.len() - "</w:styles>".len()]), "{styles}");

    // Styles the document already has don't change the style sheet.
    let mut doc = open();
    doc.format_paragraphs(
        2,
        2,
        &ParagraphFormat { style_id: Some(Some("Heading1".into())), ..ParagraphFormat::default() },
    )
    .unwrap();
    assert_eq!(part_text(&doc.save().unwrap(), "word/styles.xml"), STYLES_XML);

    // Unknown styles are refused.
    let err = doc
        .format_paragraphs(
            2,
            2,
            &ParagraphFormat { style_id: Some(Some("Fancy".into())), ..ParagraphFormat::default() },
        )
        .unwrap_err();
    assert!(err.to_string().contains("no style named Fancy"), "{err}");
}

#[test]
fn lists_reuse_existing_definitions_and_add_missing_ones() {
    let mut doc = open();
    let number = ParagraphFormat {
        list: Some(Some(ListChange { kind: ListKind::Number, level: 0 })),
        ..ParagraphFormat::default()
    };
    doc.format_paragraphs(1, 2, &number).unwrap();
    let model = round_trip(&doc);
    assert_eq!(
        paragraph(&model, 1).numbering.as_ref().map(|n| n.num_id.as_str()),
        Some("1"),
        "the document's own decimal list is reused"
    );
    assert_eq!(model.lists.len(), 1);
    assert_eq!(part_text(&doc.save().unwrap(), "word/numbering.xml").matches("<w:abstractNum ").count(), 1);

    let bullet = ParagraphFormat {
        list: Some(Some(ListChange { kind: ListKind::Bullet, level: 1 })),
        ..ParagraphFormat::default()
    };
    doc.format_paragraphs(3, 3, &bullet).unwrap();
    let model = round_trip(&doc);
    let n = paragraph(&model, 3).numbering.as_ref().unwrap();
    assert_eq!((n.num_id.as_str(), n.level), ("2", 1));
    let def = model.lists.iter().find(|l| l.num_id == "2").unwrap();
    assert_eq!((def.levels[0].format.as_str(), def.levels[1].text.as_str()), ("bullet", "◦"));

    doc.format_paragraphs(1, 1, &ParagraphFormat { list: Some(None), ..ParagraphFormat::default() }).unwrap();
    assert!(round_trip(&doc).blocks.get(1).map(|b| matches!(b, Block::Paragraph(p) if p.numbering.is_none())).unwrap());
}

#[test]
fn lists_in_a_document_without_numbering_create_the_part() {
    let mut doc = Document::new_blank();
    assert!(doc.model().lists.is_empty());
    doc.format_paragraphs(
        0,
        0,
        &ParagraphFormat {
            list: Some(Some(ListChange { kind: ListKind::Bullet, level: 0 })),
            ..ParagraphFormat::default()
        },
    )
    .unwrap();
    let saved = doc.save().unwrap();

    let names: Vec<String> = read_parts(&saved).into_iter().map(|(n, _, _)| n).collect();
    assert!(names.contains(&"word/numbering.xml".to_string()), "{names:?}");
    let types = part_text(&saved, "[Content_Types].xml");
    assert!(types.contains(r#"<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>"#), "{types}");
    let rels = part_text(&saved, "word/_rels/document.xml.rels");
    assert!(rels.contains(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml""#), "{rels}");

    let reopened = Document::open(&saved).unwrap().model();
    assert_eq!(reopened.lists.len(), 1);
    assert_eq!(paragraph(&reopened, 0).numbering.as_ref().unwrap().num_id, reopened.lists[0].num_id);

    // A second list kind goes into the same new part.
    let mut doc = Document::open(&saved).unwrap();
    doc.format_paragraphs(
        0,
        0,
        &ParagraphFormat {
            list: Some(Some(ListChange { kind: ListKind::Number, level: 0 })),
            ..ParagraphFormat::default()
        },
    )
    .unwrap();
    let saved = doc.save().unwrap();
    assert_eq!(read_parts(&saved).iter().filter(|(n, _, _)| n.contains("numbering")).count(), 1);
    assert_eq!(Document::open(&saved).unwrap().model().lists.len(), 2);
}

#[test]
fn failed_batches_roll_back_secondary_parts() {
    let mut doc = Document::new_blank();
    let before = doc.save().unwrap();
    let edits = vec![
        Edit::FormatParagraphs {
            container: Default::default(),
            first: 0,
            last: 0,
            format: ParagraphFormat {
                list: Some(Some(ListChange { kind: ListKind::Bullet, level: 0 })),
                style_id: Some(Some("Heading1".into())),
                ..ParagraphFormat::default()
            },
        },
        Edit::DeleteBlock { block: 42 },
    ];
    assert!(doc.apply(&edits).is_err());
    assert!(!doc.is_modified());
    assert!(doc.model().lists.is_empty());
    assert_eq!(read_parts(&doc.save().unwrap()).len(), read_parts(&before).len());
}

#[test]
fn paragraph_format_validation_and_range_checks() {
    let mut doc = open();
    for (first, last, format, expected) in [
        (0, 99, ParagraphFormat::default(), "not in the document"),
        (3, 5, ParagraphFormat::default(), "block 4 is not a paragraph"),
        (
            0,
            0,
            ParagraphFormat {
                indent: Some(Some(IndentChange { left: Some(99_999), ..Default::default() })),
                ..Default::default()
            },
            "indents",
        ),
        (
            0,
            0,
            ParagraphFormat {
                spacing: Some(Some(SpacingChange { line: Some(1), ..Default::default() })),
                ..Default::default()
            },
            "line spacing",
        ),
        (
            0,
            0,
            ParagraphFormat { list: Some(Some(ListChange { kind: ListKind::Number, level: 9 })), ..Default::default() },
            "list levels",
        ),
    ] {
        let err = doc.format_paragraphs(first, last, &format).unwrap_err().to_string();
        assert!(err.contains(expected), "{err}");
    }
    assert!(!doc.is_modified());
}

#[test]
fn formats_deserialize_from_json() {
    let edits: Vec<Edit> = serde_json::from_str(
        r#"[
            {"op": "splice", "first": 0, "last": 0, "content": [
                {"type": "text", "text": "Hi", "like": {"block": 0, "inline": 0}, "format": {"bold": true, "color": null, "size": 28}}
            ]},
            {"op": "format_paragraphs", "first": 0, "last": 2, "format": {
                "alignment": "justify", "style_id": null, "indent": {"left": 720}, "list": {"kind": "bullet", "level": 1}
            }}
        ]"#,
    )
    .unwrap();
    match &edits[0] {
        Edit::Splice(s) => match &s.content[0] {
            Piece::Text { format: Some(f), .. } => {
                assert_eq!(
                    (f.bold, f.color.clone(), f.size, f.font.clone()),
                    (Some(true), Some(None), Some(Some(28)), None)
                );
            }
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
    match &edits[1] {
        Edit::FormatParagraphs { format, .. } => {
            assert_eq!(format.alignment, Some(Some(Alignment::Justify)));
            assert_eq!(format.style_id, Some(None));
            assert_eq!(format.list, Some(Some(ListChange { kind: ListKind::Bullet, level: 1 })));
            assert_eq!(format.spacing, None);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        serde_json::from_str::<Vec<Edit>>(
            r#"[{"op": "format_paragraphs", "first": 0, "last": 0, "format": {"glow": true}}]"#
        )
        .is_err()
    );
}

#[test]
fn word_document_keeps_unrelated_parts_when_formatted() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/word-sample.docx")).unwrap();
    let mut doc = Document::open(&bytes).unwrap();
    let index = doc
        .model()
        .blocks
        .iter()
        .position(|b| matches!(b, Block::Paragraph(p) if p.style_id.is_none() && !p.inlines.is_empty()))
        .unwrap();
    doc.format_paragraphs(
        index,
        index,
        &ParagraphFormat {
            style_id: Some(Some("Heading2".into())),
            list: Some(Some(ListChange { kind: ListKind::Bullet, level: 0 })),
            ..ParagraphFormat::default()
        },
    )
    .unwrap();
    let saved = doc.save().unwrap();
    let reopened = Document::open(&saved).unwrap().model();
    assert_eq!(paragraph(&reopened, index).style_id.as_deref(), Some("Heading2"));
    assert!(paragraph(&reopened, index).numbering.is_some());
    let original = read_parts(&bytes);
    for (name, data, _) in read_parts(&saved) {
        if !["word/document.xml", "word/numbering.xml", "word/styles.xml"].contains(&name.as_str()) {
            let (_, before, _) = original.iter().find(|(n, _, _)| *n == name).expect("no new parts");
            assert_eq!(&data, before, "{name} changed");
        }
    }
}
