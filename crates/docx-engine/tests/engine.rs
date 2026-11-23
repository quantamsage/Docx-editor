mod common;

use common::*;
use docx_engine::model::{Alignment, Block, Indent, Inline, Numbering, RunContent, Spacing};
use docx_engine::{Document, DocxError};

fn open() -> Document {
    Document::open(&representative_docx()).expect("representative document opens")
}

fn paragraph(model: &docx_engine::DocumentModel, index: usize) -> &docx_engine::model::Paragraph {
    match &model.blocks[index] {
        Block::Paragraph(p) => p,
        other => panic!("block {index} is not a paragraph: {other:?}"),
    }
}

fn plain_text(p: &docx_engine::model::Paragraph) -> String {
    let mut out = String::new();
    for inline in &p.inlines {
        let runs = match inline {
            Inline::Run(run) => std::slice::from_ref(run),
            Inline::Hyperlink { runs, .. } => runs.as_slice(),
            Inline::Other { .. } => continue,
        };
        for run in runs {
            for item in &run.content {
                match item {
                    RunContent::Text { text } => out.push_str(text),
                    RunContent::Tab => out.push('\t'),
                    RunContent::LineBreak => out.push('\n'),
                    _ => {}
                }
            }
        }
    }
    out
}

// ----- Parsing -----

#[test]
fn parses_block_structure() {
    let model = open().model();
    let kinds: Vec<&str> = model
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(_) => "p",
            Block::Table(_) => "tbl",
            Block::Other { kind, .. } => kind.as_str(),
        })
        .collect();
    assert_eq!(kinds, ["p", "p", "p", "p", "tbl", "p", "sdt", "p", "p", "p", "p"]);
    match &model.blocks[6] {
        Block::Other { text, .. } => assert_eq!(text, "Content control text"),
        _ => unreachable!(),
    }
}

#[test]
fn parses_paragraph_and_run_formatting() {
    let model = open().model();
    let heading = paragraph(&model, 0);
    assert_eq!(heading.style_id.as_deref(), Some("Heading1"));
    assert_eq!(plain_text(heading), "Quarterly Report");

    let body = paragraph(&model, 1);
    assert_eq!(plain_text(body), "Revenue grew 12% this quarter");
    let runs: Vec<_> = body
        .inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Run(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 3, "bookmarks and proofing marks are not inlines");
    assert_eq!(runs[0].props, Default::default());
    assert_eq!(runs[1].props.bold, Some(true));
    assert_eq!(runs[1].props.color.as_deref(), Some("C00000"));
    let fancy = &runs[2].props;
    assert_eq!((fancy.italic, fancy.underline, fancy.strike), (Some(true), Some(true), Some(true)));
    assert_eq!(fancy.font.as_deref(), Some("Georgia"));
    assert_eq!(fancy.size, Some(28));
    assert_eq!(fancy.highlight.as_deref(), Some("yellow"));

    let formatted = paragraph(&model, 10);
    assert_eq!(formatted.alignment, Some(Alignment::Justify));
    assert_eq!(formatted.indent, Some(Indent { left: Some(720), right: None, first_line: Some(-360) }));
    assert_eq!(
        formatted.spacing,
        Some(Spacing { before: Some(120), after: Some(240), line: Some(360), line_rule: Some("auto".into()) })
    );
    assert_eq!(plain_text(formatted), "Terms & conditions <apply> to\tall\tusers\nSecond line");

    let list = paragraph(&model, 5);
    assert_eq!(list.numbering, Some(Numbering { num_id: "1".into(), level: 1 }));
}

#[test]
fn parses_hyperlinks_images_and_preserved_content() {
    let model = open().model();
    let links: Vec<_> = paragraph(&model, 2)
        .inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Hyperlink { url, anchor, runs } => Some((url.clone(), anchor.clone(), runs.len())),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        [(Some("https://example.com/report?q=1&r=2".into()), None, 1), (None, Some("summary".into()), 1)]
    );

    let image = paragraph(&model, 3)
        .inlines
        .iter()
        .find_map(|i| match i {
            Inline::Run(r) => r.content.iter().find_map(|c| match c {
                RunContent::Image(img) => Some(img.clone()),
                _ => None,
            }),
            _ => None,
        })
        .expect("image parsed");
    assert_eq!(image.rel_id, "rId6");
    assert_eq!(image.part.as_deref(), Some("word/media/image1.png"));
    assert_eq!((image.width_emu, image.height_emu), (Some(914_400), Some(457_200)));
    assert_eq!(image.description.as_deref(), Some("Sales chart"));
    assert!(!image.floating);

    let field = paragraph(&model, 7);
    assert_eq!(plain_text(field), "Page 1", "field codes are not visible text");
    let others: Vec<String> = paragraph(&model, 8)
        .inlines
        .iter()
        .flat_map(|i| match i {
            Inline::Other { kind, text, .. } => vec![format!("{kind}:{text}")],
            Inline::Run(r) => r
                .content
                .iter()
                .filter_map(|c| match c {
                    RunContent::Other { kind } => Some(kind.clone()),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        })
        .collect();
    // Text inside an ignorable extension element is not visible in Word either.
    assert_eq!(others, ["AlternateContent", "custom:"]);
    assert!(matches!(paragraph(&model, 9).inlines[0], Inline::Run(ref r) if r.content == [RunContent::PageBreak]));
}

#[test]
fn parses_tables_section_styles_and_parts() {
    let model = open().model();
    let Block::Table(table) = &model.blocks[4] else { panic!("expected table") };
    assert_eq!(table.style_id.as_deref(), Some("TableGrid"));
    assert_eq!(table.rows.len(), 3);
    assert_eq!(table.rows[0].cells.len(), 1);
    assert_eq!(table.rows[0].cells[0].span, 2);
    assert_eq!(table.rows[1].cells[0].vertical_merge.as_deref(), Some("restart"));
    assert_eq!(table.rows[2].cells[0].vertical_merge.as_deref(), Some("continue"));
    assert!(matches!(&table.rows[2].cells[1].blocks[0], Block::Paragraph(p) if plain_text(p) == "C"));

    let section = model.section.as_ref().expect("section");
    assert_eq!((section.page_width, section.page_height, section.landscape), (Some(15840), Some(12240), true));
    assert_eq!((section.margin_top, section.margin_left), (Some(1080), Some(1440)));
    assert!(section.title_page);

    assert_eq!(model.headers, ["word/header1.xml"]);
    assert_eq!(model.footers, ["word/footer1.xml"]);
    let style_ids: Vec<_> = model.styles.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(style_ids, ["Normal", "Heading1", "ListParagraph", "Hyperlink", "TableGrid"]);
    assert!(model.styles[0].is_default && !model.styles[1].is_default);
    assert_eq!(model.styles[1].name, "heading 1");
    assert_eq!(model.styles[1].based_on.as_deref(), Some("Normal"));
    assert!(model.warnings.is_empty());

    let stats = &model.stats;
    assert_eq!((stats.paragraphs, stats.tables, stats.images, stats.hyperlinks), (14, 1, 1, 2));
    assert!(stats.preserved_elements >= 6, "{stats:?}");
    assert!(stats.words > 20, "{stats:?}");
}

#[test]
fn model_serializes_to_json() {
    let json = serde_json::to_value(open().model()).unwrap();
    assert_eq!(json["blocks"][0]["type"], "paragraph");
    assert_eq!(json["blocks"][4]["type"], "table");
    assert_eq!(json["blocks"][1]["inlines"][1]["props"]["bold"], true);
    assert_eq!(json["section"]["landscape"], true);
}

// ----- Saving and preservation -----

#[test]
fn unedited_save_is_identical_part_by_part() {
    let original = representative_docx();
    let doc = Document::open(&original).unwrap();
    assert!(!doc.is_modified());
    let saved = doc.save().unwrap();
    assert_eq!(read_parts(&saved), read_parts(&original));
}

#[test]
fn editing_text_keeps_everything_else_intact() {
    let original = representative_docx();
    let mut doc = Document::open(&original).unwrap();
    doc.set_paragraph_text(1, "Revenue fell 3% & costs rose").unwrap();
    assert!(doc.is_modified());
    let saved = doc.save().unwrap();

    // Every other part is byte-identical, in the same order with the same compression.
    let before = read_parts(&original);
    let after = read_parts(&saved);
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        assert_eq!((&b.0, b.2), (&a.0, a.2));
        if b.0 != "word/document.xml" {
            assert_eq!(b.1, a.1, "{} changed", b.0);
        }
    }

    // In the main part, only the edited paragraph's runs changed.
    let xml = part_text(&saved, "word/document.xml");
    let edited_start = DOCUMENT_XML.find("<w:p w14:paraId=\"2B3C4D5E\"").unwrap();
    let edited_end = DOCUMENT_XML[edited_start..].find("</w:p>").unwrap() + edited_start + "</w:p>".len();
    assert!(xml.starts_with(&DOCUMENT_XML[..edited_start]), "content before the edit changed");
    assert!(xml.ends_with(&DOCUMENT_XML[edited_end..]), "content after the edit changed");
    assert!(xml.contains(
        "<w:p w14:paraId=\"2B3C4D5E\" w:rsidR=\"00A2\" w:rsidRDefault=\"00A2\"><w:bookmarkStart w:id=\"0\" w:name=\"summary\"/><w:r><w:t>Revenue fell 3% &amp; costs rose</w:t></w:r><w:proofErr w:type=\"spellStart\"/><w:bookmarkEnd w:id=\"0\"/></w:p>"
    ), "{xml}");

    let reopened = Document::open(&saved).unwrap();
    assert_eq!(plain_text(paragraph(&reopened.model(), 1)), "Revenue fell 3% & costs rose");
    let mut expected = open().model();
    let mut actual = reopened.model();
    expected.blocks.remove(1);
    actual.blocks.remove(1);
    expected.stats = Default::default();
    actual.stats = Default::default();
    assert_eq!(actual, expected, "all other blocks are unchanged");
}

#[test]
fn text_edits_keep_first_run_formatting_images_and_hyperlinks() {
    let mut doc = open();
    // Paragraph 2: plain runs around two hyperlinks.
    doc.set_paragraph_text(2, "Go to").unwrap();
    // Paragraph 3: an image run and a text run.
    doc.set_paragraph_text(3, " Chart (updated)").unwrap();
    // Paragraph 0 has no direct formatting; give a formatted paragraph new text too.
    doc.set_paragraph_text(10, "Line one\nLine\ttwo").unwrap();
    let model = Document::open(&doc.save().unwrap()).unwrap().model();

    let links = paragraph(&model, 2).inlines.iter().filter(|i| matches!(i, Inline::Hyperlink { .. })).count();
    assert_eq!(links, 2, "hyperlinks survive a text edit");
    assert_eq!(plain_text(paragraph(&model, 2)), "Go toExample sitethe summary");

    let p3 = paragraph(&model, 3);
    assert!(
        p3.inlines
            .iter()
            .any(|i| matches!(i, Inline::Run(r) if r.content.iter().any(|c| matches!(c, RunContent::Image(_))))),
        "image survives a text edit"
    );
    assert_eq!(plain_text(p3), " Chart (updated)");

    let p10 = paragraph(&model, 10);
    assert_eq!(plain_text(p10), "Line one\nLine\ttwo");
    assert_eq!(p10.alignment, Some(Alignment::Justify), "paragraph properties are kept");

    let xml = part_text(&doc.save().unwrap(), "word/document.xml");
    assert!(xml.contains("<w:t xml:space=\"preserve\"> Chart (updated)</w:t>"), "leading space is preserved");
}

#[test]
fn text_edit_keeps_formatting_of_first_text_run() {
    let mut doc = open();
    // Make the formatted run first by editing a paragraph whose first run is bold.
    doc.insert_paragraph(0, "x", None).unwrap();
    doc.set_paragraph_text(2, "replacement").unwrap();
    let saved = doc.save().unwrap();
    let model = Document::open(&saved).unwrap().model();
    let Inline::Run(run) = paragraph(&model, 2).inlines.iter().find(|i| matches!(i, Inline::Run(_))).unwrap() else {
        unreachable!()
    };
    assert_eq!(run.props, Default::default(), "first plain run had no formatting");

    // Paragraph 1 originally starts with an unformatted run; give one a format first.
    let mut doc = open();
    doc.set_paragraph_text(1, "").unwrap();
    let p = paragraph(&doc.model(), 1).clone();
    assert!(p.inlines.is_empty(), "clearing text removes the text runs: {p:?}");
}

#[test]
fn insert_delete_style_and_alignment() {
    let mut doc = open();
    let count = doc.block_count();
    doc.insert_paragraph(1, "Inserted after the title", Some("Heading1")).unwrap();
    doc.insert_paragraph(count + 1, "Appended at the end", None).unwrap();
    doc.delete_block(5).unwrap(); // the table (shifted by one)
    doc.set_paragraph_style(2, Some("ListParagraph")).unwrap();
    doc.set_paragraph_style(0, None).unwrap();
    doc.set_paragraph_alignment(2, Some(Alignment::Center)).unwrap();
    doc.set_paragraph_alignment(10, None).unwrap();

    let saved = doc.save().unwrap();
    let model = Document::open(&saved).unwrap().model();
    assert_eq!(model.blocks.len(), count + 1);
    assert_eq!(plain_text(paragraph(&model, 1)), "Inserted after the title");
    assert_eq!(paragraph(&model, 1).style_id.as_deref(), Some("Heading1"));
    assert_eq!(plain_text(paragraph(&model, model.blocks.len() - 1)), "Appended at the end");
    assert!(!model.blocks.iter().any(|b| matches!(b, Block::Table(_))));
    assert_eq!(paragraph(&model, 0).style_id, None);
    assert_eq!(paragraph(&model, 2).style_id.as_deref(), Some("ListParagraph"));
    assert_eq!(paragraph(&model, 2).alignment, Some(Alignment::Center));
    assert_eq!(paragraph(&model, 10).alignment, None);
    assert!(paragraph(&model, 10).indent.is_some(), "other properties are kept");

    let xml = part_text(&saved, "word/document.xml");
    // pStyle precedes jc inside pPr, and the appended paragraph sits before sectPr.
    assert!(xml.contains("<w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:jc w:val=\"center\"/></w:pPr>"), "{xml}");
    assert!(xml.contains("<w:p><w:r><w:t>Appended at the end</w:t></w:r></w:p><w:sectPr"));
    assert!(xml.contains("<w:spacing w:before=\"120\" w:after=\"240\" w:line=\"360\" w:lineRule=\"auto\"/><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr>"));
}

#[test]
fn alignment_is_inserted_in_schema_order() {
    let mut doc = open();
    // Paragraph 5 has pStyle and numPr; jc must follow both.
    doc.set_paragraph_alignment(5, Some(Alignment::Right)).unwrap();
    let xml = part_text(&doc.save().unwrap(), "word/document.xml");
    assert!(xml.contains(
        "<w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"1\"/><w:numId w:val=\"1\"/></w:numPr><w:jc w:val=\"right\"/></w:pPr>"
    ));
}

#[test]
fn invalid_edits_are_rejected_without_changes() {
    let mut doc = open();
    type Edit = Box<dyn Fn(&mut Document) -> docx_engine::Result<()>>;
    let cases: Vec<(Edit, &str)> = vec![
        (Box::new(|d| d.set_paragraph_text(99, "x")), "no block 99"),
        (Box::new(|d| d.set_paragraph_text(4, "x")), "not a paragraph"),
        (Box::new(|d| d.set_paragraph_text(1, "bell\u{7}")), "unsupported character"),
        (Box::new(|d| d.insert_paragraph(99, "x", None)), "past the end"),
        (Box::new(|d| d.insert_paragraph(0, "x", Some(""))), "style"),
        (Box::new(|d| d.delete_block(99)), "no block 99"),
        (Box::new(|d| d.set_paragraph_style(6, Some("Normal"))), "not a paragraph"),
    ];
    for (edit, needle) in cases {
        match edit(&mut doc) {
            Err(DocxError::InvalidEdit(message)) => assert!(message.contains(needle), "{message}"),
            other => panic!("expected InvalidEdit containing {needle:?}, got {other:?}"),
        }
    }
    assert!(!doc.is_modified());
    assert_eq!(read_parts(&doc.save().unwrap()), read_parts(&representative_docx()));
}

#[test]
fn section_break_paragraphs_cannot_be_deleted() {
    let xml = DOCUMENT_XML.replace(
        "<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>",
        "<w:p><w:pPr><w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr></w:pPr></w:p>",
    );
    let mut doc = Document::open(&docx_with(&[("word/document.xml", Some(&xml))])).unwrap();
    assert!(paragraph(&doc.model(), 9).section_break);
    assert!(matches!(doc.delete_block(9), Err(DocxError::InvalidEdit(m)) if m.contains("section")));
}

#[test]
fn deleting_the_last_block_leaves_an_empty_paragraph() {
    let mut doc = Document::new_blank();
    doc.delete_block(0).unwrap();
    assert_eq!(doc.block_count(), 1);
    let model = Document::open(&doc.save().unwrap()).unwrap().model();
    assert!(matches!(&model.blocks[..], [Block::Paragraph(p)] if p.inlines.is_empty()));
}

// ----- New documents -----

#[test]
fn blank_document_is_valid_and_editable() {
    let doc = Document::new_blank();
    let model = doc.model();
    assert_eq!(model.blocks.len(), 1);
    let section = model.section.as_ref().unwrap();
    assert_eq!((section.page_width, section.page_height, section.landscape), (Some(11906), Some(16838), false));
    assert_eq!(section.margin_left, Some(1440));
    for id in ["Normal", "Title", "Heading1", "Heading2", "Heading3", "ListParagraph", "Hyperlink", "TableGrid"] {
        assert!(model.styles.iter().any(|s| s.id == id), "missing style {id}");
    }

    let bytes = doc.save().unwrap();
    let names: Vec<_> = read_parts(&bytes).into_iter().map(|(n, _, _)| n).collect();
    assert_eq!(names[0], "[Content_Types].xml", "content types come first");
    assert!(names.contains(&"word/document.xml".to_string()) && names.contains(&"docProps/core.xml".to_string()));

    let mut reopened = Document::open(&bytes).unwrap();
    reopened.set_paragraph_text(0, "Hello").unwrap();
    reopened.insert_paragraph(0, "Title text", Some("Title")).unwrap();
    let model = Document::open(&reopened.save().unwrap()).unwrap().model();
    assert_eq!(plain_text(paragraph(&model, 0)), "Title text");
    assert_eq!(plain_text(paragraph(&model, 1)), "Hello");
}

#[test]
fn documents_with_a_default_namespace_get_prefixed_attributes() {
    let xml = format!(
        "<?xml version=\"1.0\"?><document xmlns=\"{W}\"><body><p><r><t>One</t></r></p><sectPr/></body></document>"
    );
    let mut doc = Document::open(&docx_with(&[("word/document.xml", Some(&xml))])).unwrap();
    assert_eq!(plain_text(paragraph(&doc.model(), 0)), "One");
    doc.set_paragraph_alignment(0, Some(Alignment::Center)).unwrap();
    let saved = doc.save().unwrap();
    let out = part_text(&saved, "word/document.xml");
    assert!(out.contains(&format!("xmlns:w=\"{W}\"")), "{out}");
    assert!(out.contains("<pPr><jc w:val=\"center\"/></pPr>"), "{out}");
    assert_eq!(paragraph(&Document::open(&saved).unwrap().model(), 0).alignment, Some(Alignment::Center));
}

// ----- Invalid input -----

#[test]
fn rejects_files_that_are_not_word_documents() {
    let spreadsheet_types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (docx_with(&[("[Content_Types].xml", None)]), "[Content_Types].xml"),
        (docx_with(&[("_rels/.rels", None)]), "relationships"),
        (docx_with(&[("word/document.xml", None)]), "missing"),
        (docx_with(&[("[Content_Types].xml", Some(spreadsheet_types))]), "spreadsheetml"),
        (docx_with(&[("word/document.xml", Some(&format!("<w:hdr xmlns:w=\"{W}\"/>")))]), "not a WordprocessingML"),
        (docx_with(&[("word/document.xml", Some(&format!("<w:document xmlns:w=\"{W}\"/>")))]), "no body"),
        (
            docx_with(&[(
                "_rels/.rels",
                Some("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"),
            )]),
            "main document",
        ),
    ];
    for (bytes, needle) in cases {
        match Document::open(&bytes) {
            Err(DocxError::NotWordDocument(message)) => assert!(message.contains(needle), "{message}"),
            other => panic!("expected NotWordDocument containing {needle:?}, got {:?}", other.map(|_| "a document")),
        }
    }
}

#[test]
fn rejects_damaged_and_unsafe_xml() {
    let cases = [
        DOCUMENT_XML.replace("</w:body>", ""),
        DOCUMENT_XML.replacen("<w:document", "<!DOCTYPE lol [<!ENTITY lol \"lol\">]><w:document", 1),
        "not xml at all".to_string(),
    ];
    for xml in cases {
        let err = Document::open(&docx_with(&[("word/document.xml", Some(&xml))])).unwrap_err();
        assert!(matches!(&err, DocxError::MalformedXml { part, .. } if part == "word/document.xml"), "{err:?}");
        assert!(err.to_string().contains("damaged"));
    }
}

#[test]
fn damaged_styles_are_a_warning_not_an_error() {
    let doc = Document::open(&docx_with(&[("word/styles.xml", Some("<w:styles"))])).unwrap();
    let model = doc.model();
    assert!(model.styles.is_empty());
    assert_eq!(model.warnings.len(), 1);
    assert!(model.warnings[0].contains("style definitions"));
    // The damaged part is still written back unchanged.
    assert_eq!(part_text(&doc.save().unwrap(), "word/styles.xml"), "<w:styles");
}

#[test]
fn rejects_non_zip_and_legacy_files() {
    assert!(matches!(Document::open(b"plain text"), Err(DocxError::InvalidArchive(_))));
    let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    ole.resize(512, 0);
    let err = Document::open(&ole).unwrap_err();
    assert_eq!(err, DocxError::EncryptedOrLegacy);
    assert!(err.to_string().contains("password-protected or uses the old .doc format"));
}
