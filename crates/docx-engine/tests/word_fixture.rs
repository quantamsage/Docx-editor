//! Tests against a document produced by Microsoft Word itself
//! (`tests/fixtures/word-sample.docx`, regenerated with
//! `scripts/make-word-fixture.ps1`). Edited copies of this file were checked
//! in Word with `scripts/verify-with-word.ps1`.

mod common;

use common::{part_text, read_parts};
use docx_engine::Document;
use docx_engine::model::{Alignment, Block, Inline, RunContent};

const WORD_SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/word-sample.docx");

fn texts(model: &docx_engine::DocumentModel) -> Vec<String> {
    model
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.plain_text(),
            Block::Table(_) => "<table>".into(),
            Block::Other { kind, .. } => format!("<{kind}>"),
        })
        .collect()
}

#[test]
fn reads_words_own_output() {
    let model = Document::open(WORD_SAMPLE).unwrap().model();
    let texts = texts(&model);
    assert_eq!(texts[0], "Quarterly Report");
    assert!(texts.contains(&"This paragraph is centred.".to_string()));
    assert!(texts.iter().any(|t| t.starts_with("Revenue grew twelve percent while costs were flat.")));
    assert!(texts.contains(&"<table>".to_string()));

    let styles: Vec<_> = model
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => p.style_id.clone(),
            _ => None,
        })
        .collect();
    for style in ["Title", "Heading1", "Heading2", "ListParagraph"] {
        assert!(styles.iter().any(|s| s == style), "no paragraph uses {style}");
    }
    let alignments: Vec<_> = model
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => p.alignment,
            _ => None,
        })
        .collect();
    assert!(alignments.contains(&Alignment::Center) && alignments.contains(&Alignment::Justify));

    let lists = model.blocks.iter().filter(|b| matches!(b, Block::Paragraph(p) if p.numbering.is_some())).count();
    assert_eq!(lists, 4, "two bullets and two numbered steps");

    let formatted = model.blocks.iter().find_map(|b| match b {
        Block::Paragraph(p) if p.plain_text().starts_with("Revenue grew") => Some(p),
        _ => None,
    });
    let runs: Vec<_> = formatted
        .unwrap()
        .inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Run(r) => Some(r),
            _ => None,
        })
        .collect();
    assert!(runs.iter().any(|r| r.props.bold == Some(true)));
    assert!(runs.iter().any(|r| r.props.italic == Some(true) && r.props.underline == Some(true)));
    assert!(runs.iter().any(|r| r.props.strike == Some(true)));
    assert!(runs.iter().any(|r| r.props.font.as_deref() == Some("Georgia") && r.props.size == Some(28)));
    assert!(runs.iter().any(|r| r.props.color.as_deref() == Some("FF0000")));
    assert!(runs.iter().any(|r| r.props.highlight.as_deref() == Some("yellow")));

    let stats = &model.stats;
    assert_eq!((stats.tables, stats.images, stats.hyperlinks), (1, 1, 1));
    assert!(!model.headers.is_empty() && !model.footers.is_empty());
    assert!(model.warnings.is_empty(), "{:?}", model.warnings);
    let section = model.section.unwrap();
    assert_eq!((section.page_width, section.page_height), (Some(11906), Some(16838)));
}

#[test]
fn resaving_words_output_changes_nothing() {
    let saved = Document::open(WORD_SAMPLE).unwrap().save().unwrap();
    assert_eq!(read_parts(&saved), read_parts(WORD_SAMPLE));
}

#[test]
fn editing_the_paragraph_with_a_comment_and_footnote_keeps_both() {
    let mut doc = Document::open(WORD_SAMPLE).unwrap();
    let model = doc.model();
    let index = texts(&model).iter().position(|t| t == "Reviewed paragraph.").expect("commented paragraph");
    doc.set_paragraph_text(index, "Reviewed and edited paragraph.").unwrap();
    let saved = doc.save().unwrap();

    let xml = part_text(&saved, "word/document.xml");
    for marker in ["commentRangeStart", "commentRangeEnd", "commentReference", "footnoteReference"] {
        assert!(xml.contains(marker), "{marker} was lost");
    }
    for (before, after) in read_parts(WORD_SAMPLE).iter().zip(read_parts(&saved).iter()) {
        if before.0 != "word/document.xml" {
            assert_eq!(before, after, "{} changed", before.0);
        }
    }

    let reopened = Document::open(&saved).unwrap().model();
    assert_eq!(texts(&reopened)[index], "Reviewed and edited paragraph.");
    let image_kept = reopened.blocks.iter().any(|b| match b {
        Block::Paragraph(p) => p
            .inlines
            .iter()
            .any(|i| matches!(i, Inline::Run(r) if r.content.iter().any(|c| matches!(c, RunContent::Image(_))))),
        _ => false,
    });
    assert!(image_kept);
}
