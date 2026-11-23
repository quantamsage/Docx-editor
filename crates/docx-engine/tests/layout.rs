//! Page layout: headers and footers (reading, editing, creating), page
//! numbers, page size, orientation and margins, and page breaks.

mod common;

use common::*;
use docx_engine::format::{Margins, PageSize, SectionFormat};
use docx_engine::model::{Alignment, Block, Inline, RunContent};
use docx_engine::{Container, Document, Edit, HeaderFooterKind, InlineRef, Piece, Splice};

fn word_sample() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/word-sample.docx")).unwrap()
}

fn texts(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.plain_text(),
            _ => String::from("<block>"),
        })
        .collect()
}

fn fields(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for block in blocks {
        if let Block::Paragraph(p) = block {
            for inline in &p.inlines {
                if let Inline::Other { field: Some(f), .. } = inline {
                    out.push(f.clone());
                }
            }
        }
    }
    out
}

fn section_format(f: SectionFormat) -> Edit {
    Edit::FormatSection { format: f }
}

#[test]
fn headers_and_footers_are_in_the_model() {
    let model = Document::open(&representative_docx()).unwrap().model();
    let header = model.header.unwrap();
    assert_eq!(header.part, "word/header1.xml");
    assert_eq!(texts(&header.blocks), ["Company Confidential"]);
    let footer = model.footer.unwrap();
    assert_eq!(fields(&footer.blocks), ["PAGE"]);

    // Word's own document: default, first-page and even-page headers; the
    // model shows the default ones.
    let model = Document::open(&word_sample()).unwrap().model();
    assert_eq!(model.header.as_ref().unwrap().part, "word/header2.xml");
    assert_eq!(texts(&model.header.unwrap().blocks), ["Company Confidential"]);
    assert_eq!(model.footer.unwrap().part, "word/footer2.xml");
    assert_eq!(model.stats.words, Document::open(&word_sample()).unwrap().model().stats.words);

    assert!(Document::new_blank().model().header.is_none());
}

#[test]
fn header_text_is_edited_in_its_own_part() {
    let bytes = word_sample();
    let mut doc = Document::open(&bytes).unwrap();
    doc.splice(&Splice {
        container: Container::part("word/header2.xml"),
        first: 0,
        last: 0,
        content: vec![Piece::Text {
            text: "Internal use only".into(),
            like: Some(InlineRef { block: 0, inline: 0 }),
            format: None,
        }],
    })
    .unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    assert_eq!(changed_parts(&bytes, &saved), ["word/header2.xml"]);
    let header = Document::open(&saved).unwrap().model().header.unwrap();
    assert_eq!(texts(&header.blocks), ["Internal use only"]);
    assert!(
        part_text(&saved, "word/header2.xml")
            .contains(r#"<w:pPr><w:pStyle w:val="Header"/></w:pPr><w:r><w:t>Internal use only</w:t></w:r>"#)
    );

    for part in ["word/styles.xml", "word/header9.xml", "../word/header2.xml"] {
        let err =
            doc.splice(&Splice { container: Container::part(part), first: 0, last: 0, content: vec![] }).unwrap_err();
        assert!(err.to_string().contains("no header or footer"), "{part}: {err}");
    }
}

#[test]
fn adding_a_header_creates_part_relationship_and_reference() {
    let mut doc = Document::new_blank();
    doc.apply(&[Edit::AddHeaderFooter { kind: HeaderFooterKind::Header }]).unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    assert!(part_text(&saved, "[Content_Types].xml").contains(r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#));
    let rels = part_text(&saved, "word/_rels/document.xml.rels");
    assert!(rels.contains(r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#), "{rels}");
    let xml = part_text(&saved, "word/document.xml");
    assert!(xml.contains(r#"<w:sectPr><w:headerReference w:type="default" r:id="rId3"/><w:pgSz"#), "{xml}");
    let header = part_text(&saved, "word/header1.xml");
    assert!(header.contains(r#"<w:p><w:pPr><w:pStyle w:val="Header"/></w:pPr></w:p>"#), "{header}");

    let model = Document::open(&saved).unwrap().model();
    assert_eq!(model.header.unwrap().part, "word/header1.xml");
    assert!(model.styles.iter().any(|s| s.id == "Header"));
    // Adding it again changes nothing.
    let mut again = Document::open(&saved).unwrap();
    again.apply(&[Edit::AddHeaderFooter { kind: HeaderFooterKind::Header }]).unwrap();
    assert!(!again.save().unwrap().is_empty());
    assert_eq!(changed_parts(&saved, &again.save().unwrap()), Vec::<String>::new());
}

#[test]
fn every_section_gets_the_new_footer() {
    // Two sections, neither with headers or footers.
    let xml = DOCUMENT_XML
        .replace(
            r#"<w:p><w:r><w:t xml:space="preserve">Visit </w:t>"#,
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:pPr><w:r><w:t xml:space="preserve">Visit </w:t>"#,
        )
        .replace(r#"<w:headerReference w:type="default" r:id="rId7"/><w:footerReference w:type="default" r:id="rId8"/>"#, "");
    let mut doc = Document::open(&docx_with(&[("word/document.xml", Some(&xml))])).unwrap();
    assert!(doc.model().footer.is_none());
    doc.apply(&[Edit::InsertPageNumbers {
        kind: HeaderFooterKind::Footer,
        alignment: Alignment::Right,
        with_total: true,
    }])
    .unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    let main = part_text(&saved, "word/document.xml");
    assert_eq!(main.matches(r#"<w:footerReference w:type="default" r:id="rId10"/>"#).count(), 2, "{main}");
    let footer = Document::open(&saved).unwrap().model().footer.unwrap();
    assert_eq!(footer.part, "word/footer2.xml", "footer1.xml is taken");
    assert_eq!(texts(&footer.blocks), ["Page 1 of 1"]);
    assert_eq!(fields(&footer.blocks), ["PAGE", "NUMPAGES"]);
    let Block::Paragraph(p) = &footer.blocks[0] else { panic!() };
    assert_eq!(p.alignment, Some(Alignment::Right));
}

#[test]
fn page_numbers_go_into_the_footer_once() {
    let mut doc = Document::new_blank();
    doc.apply(&[Edit::InsertPageNumbers {
        kind: HeaderFooterKind::Footer,
        alignment: Alignment::Center,
        with_total: false,
    }])
    .unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    let footer = part_text(&saved, "word/footer1.xml");
    assert!(
        footer.contains(r#"<w:p><w:pPr><w:pStyle w:val="Footer"/><w:jc w:val="center"/></w:pPr><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>"#),
        "the empty paragraph of the new footer is used: {footer}"
    );
    let err = doc
        .apply(&[Edit::InsertPageNumbers {
            kind: HeaderFooterKind::Footer,
            alignment: Alignment::Center,
            with_total: false,
        }])
        .unwrap_err();
    assert!(err.to_string().contains("already has page numbers"), "{err}");

    // Word's document already numbers its pages (a complex PAGE field).
    let mut word = Document::open(&word_sample()).unwrap();
    assert!(
        word.apply(&[Edit::InsertPageNumbers {
            kind: HeaderFooterKind::Footer,
            alignment: Alignment::Center,
            with_total: false
        }])
        .is_err()
    );
    // Its header gets them.
    word.apply(&[Edit::InsertPageNumbers {
        kind: HeaderFooterKind::Header,
        alignment: Alignment::Right,
        with_total: false,
    }])
    .unwrap();
    let header = word.model().header.unwrap();
    assert_eq!(texts(&header.blocks), ["Company Confidential", "1"]);
}

#[test]
fn page_size_orientation_and_margins() {
    let bytes = representative_docx();
    let mut doc = Document::open(&bytes).unwrap(); // Letter, landscape
    doc.apply(&[section_format(SectionFormat { landscape: Some(false), ..Default::default() })]).unwrap();
    let s = doc.model().section.unwrap();
    assert_eq!((s.page_width, s.page_height, s.landscape), (Some(12240), Some(15840), false));

    doc.apply(&[section_format(SectionFormat {
        size: Some(PageSize { width: 11906, height: 16838 }),
        margins: Some(Margins { top: Some(720), bottom: Some(720), ..Default::default() }),
        page_number_start: Some(Some(5)),
        ..Default::default()
    })])
    .unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    assert_eq!(changed_parts(&bytes, &saved), ["word/document.xml"]);
    let s = Document::open(&saved).unwrap().model().section.unwrap();
    assert_eq!((s.page_width, s.page_height, s.landscape), (Some(11906), Some(16838), false));
    assert_eq!((s.margin_top, s.margin_bottom, s.margin_left), (Some(720), Some(720), Some(1440)));
    assert_eq!(s.page_number_start, Some(5));
    let xml = part_text(&saved, "word/document.xml");
    assert!(xml.contains(r#"<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="720" w:right="1440" w:bottom="720" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:pgNumType w:start="5"/><w:cols w:space="720"/>"#), "{xml}");

    doc.apply(&[section_format(SectionFormat {
        landscape: Some(true),
        page_number_start: Some(None),
        ..Default::default()
    })])
    .unwrap();
    let s = doc.model().section.unwrap();
    assert_eq!((s.page_width, s.page_height, s.landscape, s.page_number_start), (Some(16838), Some(11906), true, None));

    for (format, expected) in [
        (
            SectionFormat {
                margins: Some(Margins { left: Some(9000), right: Some(9000), ..Default::default() }),
                ..Default::default()
            },
            "no room",
        ),
        (SectionFormat { size: Some(PageSize { width: 100, height: 16838 }), ..Default::default() }, "page sizes"),
        (
            SectionFormat { margins: Some(Margins { top: Some(40000), ..Default::default() }), ..Default::default() },
            "margins must",
        ),
    ] {
        let err = doc.apply(&[section_format(format)]).unwrap_err();
        assert!(err.to_string().contains(expected), "{err}");
    }
}

#[test]
fn page_setup_applies_to_every_section_and_creates_missing_properties() {
    let xml = DOCUMENT_XML.replace(
        r#"<w:p><w:r><w:t xml:space="preserve">Visit </w:t>"#,
        r#"<w:p><w:pPr><w:sectPr/></w:pPr><w:r><w:t xml:space="preserve">Visit </w:t>"#,
    );
    let mut doc = Document::open(&docx_with(&[("word/document.xml", Some(&xml))])).unwrap();
    doc.apply(&[section_format(SectionFormat {
        size: Some(PageSize { width: 12240, height: 15840 }),
        landscape: Some(false),
        margins: Some(Margins { left: Some(1080), ..Default::default() }),
        ..Default::default()
    })])
    .unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    let xml = part_text(&saved, "word/document.xml");
    assert!(
        xml.contains(r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1080" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>"#),
        "the empty section gets complete properties: {xml}"
    );
    assert_eq!(xml.matches(r#"<w:pgSz w:w="12240" w:h="15840"/>"#).count(), 2);
}

#[test]
fn page_breaks_split_the_paragraph() {
    let mut doc = Document::new_blank();
    doc.splice(&Splice {
        container: Container::body(),
        first: 0,
        last: 0,
        content: vec![
            Piece::Text { text: "Before".into(), like: None, format: None },
            Piece::PageBreak,
            Piece::Break,
            Piece::Text { text: "After".into(), like: None, format: None },
        ],
    })
    .unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    assert!(part_text(&saved, "word/document.xml").contains(r#"<w:p><w:r><w:t>Before</w:t></w:r><w:r><w:br w:type="page"/></w:r></w:p><w:p><w:r><w:t>After</w:t></w:r></w:p>"#));
    let model = Document::open(&saved).unwrap().model();
    let Block::Paragraph(p) = &model.blocks[0] else { panic!() };
    assert!(matches!(&p.inlines[1], Inline::Run(r) if r.content == [RunContent::PageBreak]));
    assert_eq!(texts(&model.blocks), ["Before", "After"]);
}

#[test]
fn layout_edits_deserialize_from_json() {
    let edits: Vec<Edit> = serde_json::from_str(
        r#"[
            {"op": "format_section", "format": {"size": {"width": 12240, "height": 15840}, "landscape": true, "margins": {"top": 720}, "page_number_start": null}},
            {"op": "add_header_footer", "kind": "header"},
            {"op": "insert_page_numbers", "kind": "footer"},
            {"op": "splice", "container": {"part": "word/header1.xml"}, "first": 0, "last": 0, "content": [{"type": "page_break"}, {"type": "image", "image": "img1", "width_emu": 9525, "height_emu": 9525}]}
        ]"#,
    )
    .unwrap();
    match &edits[2] {
        Edit::InsertPageNumbers { alignment, with_total, .. } => {
            assert_eq!((*alignment, *with_total), (Alignment::Center, false))
        }
        other => panic!("{other:?}"),
    }
    match &edits[0] {
        Edit::FormatSection { format } => assert_eq!(format.page_number_start, Some(None)),
        other => panic!("{other:?}"),
    }
}
