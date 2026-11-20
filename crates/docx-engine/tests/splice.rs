//! Tests for `Document::splice` and `Document::apply`: the edits behind
//! typing, Enter, joining paragraphs, deleting across paragraphs and
//! pasting.

mod common;

use common::*;
use docx_engine::model::{Alignment, Block, Inline, Paragraph};
use docx_engine::{Document, DocumentModel, Edit, InlineRef, Piece, Splice};

fn open() -> Document {
    Document::open(&representative_docx()).expect("representative document opens")
}

fn paragraph(model: &DocumentModel, index: usize) -> &Paragraph {
    match &model.blocks[index] {
        Block::Paragraph(p) => p,
        other => panic!("block {index} is not a paragraph: {other:?}"),
    }
}

fn text(model: &DocumentModel, index: usize) -> String {
    paragraph(model, index).plain_text()
}

fn keep(block: usize, inline: usize) -> Piece {
    Piece::Keep { block, inline }
}

fn t(text: &str, like: Option<(usize, usize)>) -> Piece {
    Piece::Text { text: text.into(), like: like.map(|(block, inline)| InlineRef { block, inline }) }
}

fn splice(doc: &mut Document, first: usize, last: usize, content: Vec<Piece>) -> docx_engine::Result<()> {
    doc.splice(&Splice { first, last, content })
}

fn main_xml(doc: &Document) -> String {
    part_text(&doc.save().unwrap(), "word/document.xml")
}

/// Saves, reopens and returns the reopened document's model.
fn round_trip(doc: &Document) -> DocumentModel {
    Document::open(&doc.save().unwrap()).expect("saved document reopens").model()
}

fn run_props(p: &Paragraph, inline: usize) -> &docx_engine::model::RunProps {
    match &p.inlines[inline] {
        Inline::Run(r) => &r.props,
        other => panic!("inline {inline} is not a run: {other:?}"),
    }
}

#[test]
fn typing_inside_a_formatted_run_keeps_neighbours_and_markers() {
    let mut doc = open();
    // "Revenue grew " + **12%** + _ this quarter_ → change 12% to 15.5%.
    splice(&mut doc, 1, 1, vec![keep(1, 0), t("15.5%", Some((1, 1))), keep(1, 2)]).unwrap();
    let model = round_trip(&doc);
    let p = paragraph(&model, 1);
    assert_eq!(p.plain_text(), "Revenue grew 15.5% this quarter");
    assert_eq!(p.inlines.len(), 3);
    let bold = run_props(p, 1);
    assert_eq!((bold.bold, bold.color.as_deref()), (Some(true), Some("C00000")));
    let italic = run_props(p, 2);
    assert_eq!(
        (italic.italic, italic.font.as_deref(), italic.highlight.as_deref()),
        (Some(true), Some("Georgia"), Some("yellow"))
    );

    let xml = main_xml(&doc);
    // Kept runs and the paragraph's start tag keep their exact markup.
    assert!(xml.contains(r#"<w:p w14:paraId="2B3C4D5E" w:rsidR="00A2" w:rsidRDefault="00A2"><w:bookmarkStart w:id="0" w:name="summary"/><w:r><w:t xml:space="preserve">Revenue grew </w:t></w:r>"#));
    assert!(xml.contains(r#"<w:t xml:space="preserve"> this quarter</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>"#));
    assert!(xml.contains(r#"<w:proofErr w:type="spellStart"/>"#));
    // New text sits where the replaced run was, before the proofing mark.
    assert!(xml.contains(r#"<w:t>15.5%</w:t></w:r><w:proofErr"#), "{xml}");
    // Everything outside the paragraph is untouched.
    assert!(xml.contains(r#"<w:hyperlink r:id="rId5" w:history="1">"#));
    assert!(xml.contains(r#"<w:sdt><w:sdtPr><w:alias w:val="Client"/>"#));
}

#[test]
fn enter_splits_a_paragraph_and_continues_its_formatting() {
    let mut doc = open();
    let blocks = doc.block_count();
    splice(&mut doc, 1, 1, vec![keep(1, 0), t("1", Some((1, 1))), Piece::Break, t("2%", Some((1, 1))), keep(1, 2)])
        .unwrap();
    assert_eq!(doc.block_count(), blocks + 1);
    let model = round_trip(&doc);
    assert_eq!(text(&model, 1), "Revenue grew 1");
    assert_eq!(text(&model, 2), "2% this quarter");
    assert_eq!(run_props(paragraph(&model, 2), 0).bold, Some(true));

    let xml = main_xml(&doc);
    let second = xml.split("Revenue grew ").nth(1).unwrap();
    // Markers after the split point move with the text after it.
    let first_end = second.find("</w:p>").unwrap();
    assert!(!second[..first_end].contains("bookmarkEnd"));
    assert!(second[first_end..].contains("bookmarkEnd"));
    // The new paragraph doesn't duplicate the first one's paraId.
    assert_eq!(xml.matches("2B3C4D5E").count(), 1);

    // A heading split in two gives two headings.
    let mut doc = open();
    splice(&mut doc, 0, 0, vec![t("Quarterly", Some((0, 0))), Piece::Break, t(" Report", Some((0, 0)))]).unwrap();
    let model = round_trip(&doc);
    assert_eq!((text(&model, 0).as_str(), text(&model, 1).as_str()), ("Quarterly", " Report"));
    assert_eq!(paragraph(&model, 1).style_id.as_deref(), Some("Heading1"));

    // Enter at the end of a justified paragraph creates an empty one with the same properties.
    let mut doc = open();
    let last = doc.block_count() - 1;
    splice(&mut doc, last, last, vec![keep(last, 0), Piece::Break]).unwrap();
    let model = round_trip(&doc);
    let new = paragraph(&model, last + 1);
    assert!(new.inlines.is_empty());
    assert_eq!(new.alignment, Some(Alignment::Justify));
    assert_eq!(new.spacing.as_ref().and_then(|s| s.after), Some(240));
}

#[test]
fn joining_paragraphs_keeps_both_contents_and_the_first_style() {
    let mut doc = open();
    let blocks = doc.block_count();
    splice(&mut doc, 0, 1, vec![keep(0, 0), keep(1, 0), keep(1, 1), keep(1, 2)]).unwrap();
    assert_eq!(doc.block_count(), blocks - 1);
    let model = round_trip(&doc);
    assert_eq!(text(&model, 0), "Quarterly ReportRevenue grew 12% this quarter");
    assert_eq!(paragraph(&model, 0).style_id.as_deref(), Some("Heading1"));
    let xml = main_xml(&doc);
    assert!(
        xml.contains(r#"<w:bookmarkStart w:id="0" w:name="summary"/>"#) && xml.contains(r#"<w:bookmarkEnd w:id="0"/>"#)
    );
    assert!(xml.contains(r#"<w:p w14:paraId="1A2B3C4D" w:rsidR="00A1">"#), "the first paragraph's element is kept");
}

#[test]
fn deleting_across_paragraphs_keeps_hyperlinks_and_bookmarks() {
    let mut doc = open();
    // Select from inside "Revenue grew …" to just before "the summary" link.
    splice(&mut doc, 1, 2, vec![keep(1, 0), t("and ", Some((1, 0))), keep(2, 3)]).unwrap();
    let model = round_trip(&doc);
    let p = paragraph(&model, 1);
    assert_eq!(p.plain_text(), "Revenue grew and the summary");
    assert!(matches!(&p.inlines[2], Inline::Hyperlink { anchor: Some(a), .. } if a == "summary"));
    let xml = main_xml(&doc);
    assert!(
        xml.contains(r#"<w:bookmarkStart w:id="0" w:name="summary"/>"#) && xml.contains(r#"<w:bookmarkEnd w:id="0"/>"#)
    );
    assert!(!xml.contains("Example site"), "the removed hyperlink is gone");
    // The external hyperlink's relationship part is untouched.
    let rels = part_text(&doc.save().unwrap(), "word/_rels/document.xml.rels");
    assert_eq!(rels, DOCUMENT_RELS);
}

#[test]
fn images_can_be_kept_or_deleted() {
    let mut doc = open();
    splice(&mut doc, 3, 3, vec![keep(3, 0), t(" Figure 2", Some((3, 1)))]).unwrap();
    let model = round_trip(&doc);
    assert_eq!(model.stats.images, 1);
    assert_eq!(text(&model, 3), " Figure 2");

    splice(&mut doc, 3, 3, vec![t("No picture", None)]).unwrap();
    let model = round_trip(&doc);
    assert_eq!(model.stats.images, 0);
    assert_eq!(text(&model, 3), "No picture");
    // The media part stays in the package (Word ignores unreferenced parts).
    assert!(read_parts(&doc.save().unwrap()).iter().any(|(n, _, _)| n == "word/media/image1.png"));
}

#[test]
fn pasting_lines_creates_paragraphs_with_the_same_properties() {
    let mut doc = open();
    let last = doc.block_count() - 1;
    splice(
        &mut doc,
        last,
        last,
        vec![keep(last, 0), Piece::Break, t("Pasted one", None), Piece::Break, t("Pasted\ttwo\nline", None)],
    )
    .unwrap();
    let model = round_trip(&doc);
    assert_eq!(text(&model, last + 1), "Pasted one");
    assert_eq!(text(&model, last + 2), "Pasted\ttwo\nline");
    assert_eq!(paragraph(&model, last + 2).alignment, Some(Alignment::Justify));
    assert!(model.section.is_some(), "the body's final section properties are kept");
}

#[test]
fn unsupported_inline_content_is_kept_when_editing_around_it() {
    let mut doc = open();
    // Field paragraph: "Page " + field runs + comment reference. Edit "Page ".
    let model = doc.model();
    let n = paragraph(&model, 7).inlines.len();
    let mut content = vec![t("Seite ", Some((7, 0)))];
    content.extend((1..n).map(|i| keep(7, i)));
    splice(&mut doc, 7, 7, content).unwrap();
    let xml = main_xml(&doc);
    assert!(xml.contains("<w:t xml:space=\"preserve\">Seite </w:t>"));
    assert!(xml.contains(r#"<w:instrText xml:space="preserve"> PAGE </w:instrText>"#));
    assert!(xml.contains(r#"<w:commentReference w:id="0"/>"#));
    // Alternate content and extension elements in another paragraph are untouched.
    assert!(xml.contains(r#"<x:custom x:flag="1">Extension text</x:custom>"#));
}

#[test]
fn invalid_splices_are_rejected_without_changes() {
    let original = representative_docx();
    let cases: Vec<(usize, usize, Vec<Piece>, &str)> = vec![
        (2, 1, vec![], "are not in the document"),
        (1, 99, vec![], "are not in the document"),
        (3, 5, vec![], "block 4 is not a paragraph"),
        (1, 1, vec![keep(1, 2), keep(1, 0)], "document order"),
        (1, 1, vec![keep(1, 0), keep(1, 0)], "document order"),
        (1, 1, vec![keep(2, 0)], "not part of the edited paragraphs"),
        (1, 1, vec![keep(1, 9)], "not part of the edited paragraphs"),
        (2, 2, vec![t("x", Some((2, 1)))], "not a run"),
        (1, 1, vec![t("bad\u{1}char", None)], "unsupported character"),
    ];
    for (first, last, content, expected) in cases {
        let mut doc = open();
        let err = splice(&mut doc, first, last, content.clone()).unwrap_err().to_string();
        assert!(err.contains(expected), "{first}..{last} {content:?}: {err}");
        assert!(!doc.is_modified());
        assert_eq!(doc.save().unwrap(), Document::open(&original).unwrap().save().unwrap());
    }
}

const SECTIONS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:jc w:val="center"/><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr></w:pPr><w:r><w:t>Section one</w:t></w:r></w:p><w:p><w:r><w:t>Section two</w:t></w:r></w:p><w:p><w:pPr><w:sectPr><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/></w:sectPr></w:pPr><w:r><w:t>End of two</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:body></w:document>"#;

#[test]
fn section_breaks_are_never_lost() {
    let open_sections = || Document::open(&docx_with(&[("word/document.xml", Some(SECTIONS_XML))])).unwrap();

    // Joining across a section break is refused.
    let mut doc = open_sections();
    let err = splice(&mut doc, 0, 1, vec![keep(0, 0), keep(1, 0)]).unwrap_err().to_string();
    assert!(err.contains("ends a section"), "{err}");

    // Splitting a section-ending paragraph moves the break to the second half.
    let mut doc = open_sections();
    splice(&mut doc, 0, 0, vec![t("Section", Some((0, 0))), Piece::Break, t(" one", Some((0, 0)))]).unwrap();
    let xml = main_xml(&doc);
    let paragraphs: Vec<&str> = xml.split("<w:p>").skip(1).collect();
    assert!(!paragraphs[0].contains("sectPr") && paragraphs[0].contains(r#"<w:jc w:val="center"/>"#));
    assert!(
        paragraphs[1].contains(r#"<w:jc w:val="center"/><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>"#),
        "{xml}"
    );

    // Joining into a section-ending paragraph keeps that paragraph's break.
    let mut doc = open_sections();
    splice(&mut doc, 1, 2, vec![keep(1, 0), keep(2, 0)]).unwrap();
    let xml = main_xml(&doc);
    assert!(xml.contains(r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/></w:sectPr></w:pPr><w:r><w:t>Section two</w:t></w:r><w:r><w:t>End of two</w:t></w:r></w:p>"#), "{xml}");
    assert_eq!(xml.matches("<w:sectPr>").count(), 3);
}

#[test]
fn apply_is_atomic() {
    let mut doc = open();
    let before = doc.model();
    let edits = vec![
        Edit::Splice(Splice { first: 0, last: 0, content: vec![t("Changed", Some((0, 0)))] }),
        Edit::DeleteBlock { block: 99 },
    ];
    let err = doc.apply(&edits).unwrap_err().to_string();
    assert!(err.contains("edit 2 of 2"), "{err}");
    assert_eq!(doc.model(), before);
    assert!(!doc.is_modified());

    doc.apply(&edits[..1]).unwrap();
    assert_eq!(text(&doc.model(), 0), "Changed");
    assert!(doc.is_modified());
}

#[test]
fn edits_deserialize_from_json() {
    let edits: Vec<Edit> = serde_json::from_str(
        r#"[
            {"op": "splice", "first": 1, "last": 2, "content": [
                {"type": "keep", "block": 1, "inline": 0},
                {"type": "text", "text": "hi", "like": {"block": 1, "inline": 1}},
                {"type": "text", "text": "plain"},
                {"type": "break"}
            ]},
            {"op": "insert_paragraph", "at": 0},
            {"op": "delete_block", "block": 3},
            {"op": "set_paragraph_style", "block": 0, "style_id": null},
            {"op": "set_paragraph_alignment", "block": 0, "alignment": "justify"}
        ]"#,
    )
    .unwrap();
    assert_eq!(
        edits[0],
        Edit::Splice(Splice {
            first: 1,
            last: 2,
            content: vec![keep(1, 0), t("hi", Some((1, 1))), t("plain", None), Piece::Break],
        })
    );
    assert_eq!(edits[4], Edit::SetParagraphAlignment { block: 0, alignment: Some(Alignment::Justify) });
    assert!(serde_json::from_str::<Vec<Edit>>(r#"[{"op": "format_disk"}]"#).is_err());
}

#[test]
fn word_document_survives_splices() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/word-sample.docx")).unwrap();
    let mut doc = Document::open(&bytes).unwrap();
    let model = doc.model();
    let (index, p) = model
        .blocks
        .iter()
        .enumerate()
        .find_map(|(i, b)| match b {
            Block::Paragraph(p) if p.inlines.iter().any(|x| matches!(x, Inline::Run(_))) => Some((i, p)),
            _ => None,
        })
        .expect("a paragraph with runs");
    let mut content: Vec<Piece> = (0..p.inlines.len()).map(|i| keep(index, i)).collect();
    content.push(t(" (edited)", None));
    content.push(Piece::Break);
    content.push(t("A new paragraph.", None));
    splice(&mut doc, index, index, content).unwrap();

    let saved = doc.save().unwrap();
    let reopened = Document::open(&saved).unwrap().model();
    assert!(paragraph(&reopened, index).plain_text().ends_with(" (edited)"));
    assert_eq!(paragraph(&reopened, index + 1).plain_text(), "A new paragraph.");
    // Every other part is byte-identical.
    let original = read_parts(&bytes);
    for (name, data, _) in read_parts(&saved) {
        if name != "word/document.xml" {
            let (_, before, _) = original.iter().find(|(n, _, _)| *n == name).expect("part existed");
            assert_eq!(&data, before, "{name} changed");
        }
    }
    assert_eq!(reopened.stats.images, model.stats.images);
    assert_eq!(reopened.stats.tables, model.stats.tables);
    assert_eq!(reopened.stats.hyperlinks, model.stats.hyperlinks);
}
