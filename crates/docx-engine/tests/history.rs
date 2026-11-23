//! Undo and redo: snapshots restore every kind of edit exactly, typing in
//! a group is one step, failed batches leave no step, and undoing all
//! edits gives back the original file.

mod common;

use common::*;
use docx_engine::history::{History, MAX_STEPS};
use docx_engine::model::{Alignment, Block};
use docx_engine::{Container, Document, Edit, HeaderFooterKind, InlineRef, Piece, Splice};

fn text(doc: &Document, block: usize) -> String {
    match &doc.model().blocks[block] {
        Block::Paragraph(p) => p.plain_text(),
        other => panic!("{other:?}"),
    }
}

fn typing(block: usize, text: &str) -> Edit {
    Edit::Splice(Splice {
        container: Container::body(),
        first: block,
        last: block,
        content: vec![Piece::Text { text: text.into(), like: Some(InlineRef { block, inline: 0 }), format: None }],
    })
}

/// Records, then applies; forgets the step if the batch fails.
fn apply(history: &mut History, doc: &mut Document, edits: &[Edit], group: Option<&str>) -> docx_engine::Result<()> {
    let recorded = history.record(doc, group);
    let result = doc.apply(edits);
    if result.is_err() && recorded {
        history.forget_last();
    }
    result
}

#[test]
fn undo_and_redo_restore_text() {
    let mut doc = Document::open(&representative_docx()).unwrap();
    let mut history = History::new();
    assert!(!history.can_undo());
    apply(&mut history, &mut doc, &[typing(0, "First")], None).unwrap();
    apply(&mut history, &mut doc, &[typing(0, "Second")], None).unwrap();
    assert_eq!(text(&doc, 0), "Second");

    assert!(history.undo(&mut doc).unwrap());
    assert_eq!(text(&doc, 0), "First");
    assert!(history.can_redo());
    assert!(history.undo(&mut doc).unwrap());
    assert_eq!(text(&doc, 0), "Quarterly Report");
    assert!(!doc.is_modified(), "back to the document as opened");
    assert!(!history.undo(&mut doc).unwrap(), "nothing more to undo");

    assert!(history.redo(&mut doc).unwrap());
    assert!(history.redo(&mut doc).unwrap());
    assert_eq!(text(&doc, 0), "Second");
    assert!(!history.redo(&mut doc).unwrap());

    // A new edit after undoing clears the redo steps.
    history.undo(&mut doc).unwrap();
    apply(&mut history, &mut doc, &[typing(0, "Third")], None).unwrap();
    assert!(!history.can_redo());
}

#[test]
fn typing_in_a_group_is_one_step() {
    let mut doc = Document::open(&representative_docx()).unwrap();
    let mut history = History::new();
    for word in ["Q", "Qu", "Qua"] {
        apply(&mut history, &mut doc, &[typing(0, word)], Some("typing-1")).unwrap();
    }
    apply(&mut history, &mut doc, &[typing(0, "Quar")], Some("typing-2")).unwrap();
    history.undo(&mut doc).unwrap();
    assert_eq!(text(&doc, 0), "Qua");
    history.undo(&mut doc).unwrap();
    assert_eq!(text(&doc, 0), "Quarterly Report", "the first group was one step");

    // After an undo, the same group starts a new step.
    history.redo(&mut doc).unwrap();
    apply(&mut history, &mut doc, &[typing(0, "Quax")], Some("typing-1")).unwrap();
    history.undo(&mut doc).unwrap();
    assert_eq!(text(&doc, 0), "Qua");
}

#[test]
fn failed_batches_leave_no_step() {
    let mut doc = Document::open(&representative_docx()).unwrap();
    let mut history = History::new();
    assert!(apply(&mut history, &mut doc, &[Edit::DeleteBlock { block: 99 }], None).is_err());
    assert!(!history.can_undo());
}

#[test]
fn undo_restores_secondary_parts_and_pictures() {
    let original = representative_docx();
    let mut doc = Document::open(&original).unwrap();
    let mut history = History::new();
    let image = doc.stage_image(PNG.to_vec()).unwrap().id;
    let steps: Vec<Vec<Edit>> = vec![
        vec![Edit::InsertPageNumbers { kind: HeaderFooterKind::Header, alignment: Alignment::Right, with_total: true }],
        vec![Edit::Splice(Splice {
            container: Container::body(),
            first: 0,
            last: 0,
            content: vec![
                Piece::Keep { block: 0, inline: 0 },
                Piece::Image { image, width_emu: 9525, height_emu: 9525, description: None },
            ],
        })],
        vec![Edit::InsertTable { container: Container::body(), at: 1, rows: 2, columns: 2 }],
        vec![Edit::FormatParagraphs {
            container: Container::body(),
            first: 2,
            last: 2,
            format: docx_engine::format::ParagraphFormat {
                style_id: Some(Some("Title".into())),
                list: Some(Some(docx_engine::format::ListChange {
                    kind: docx_engine::format::ListKind::Bullet,
                    level: 0,
                })),
                ..Default::default()
            },
        }],
        vec![Edit::FormatSection {
            format: docx_engine::format::SectionFormat { landscape: Some(false), ..Default::default() },
        }],
    ];
    let mut saved = Vec::new();
    for edits in &steps {
        apply(&mut history, &mut doc, edits, None).unwrap();
        saved.push(doc.save().unwrap());
    }
    assert_eq!(doc.model().stats.images, 2);

    for i in (0..steps.len()).rev() {
        history.undo(&mut doc).unwrap();
        let now = doc.save().unwrap();
        let expected = if i == 0 { &original } else { &saved[i - 1] };
        assert_eq!(changed_parts(expected, &now), Vec::<String>::new(), "after undoing step {}", i + 1);
    }
    assert!(!doc.is_modified());

    // Redo all of them: the same files again.
    for (i, expected) in saved.iter().enumerate() {
        history.redo(&mut doc).unwrap();
        assert_eq!(changed_parts(expected, &doc.save().unwrap()), Vec::<String>::new(), "after redoing step {}", i + 1);
    }
    assert_valid_docx(&doc.save().unwrap());
}

#[test]
fn history_is_bounded() {
    let mut doc = Document::new_blank();
    let mut history = History::new();
    for i in 0..MAX_STEPS + 5 {
        apply(&mut history, &mut doc, &[Edit::InsertParagraph { at: 0, text: format!("{i}"), style_id: None }], None)
            .unwrap();
    }
    let mut undone = 0;
    while history.undo(&mut doc).unwrap() {
        undone += 1;
    }
    assert_eq!(undone, MAX_STEPS);
    assert_eq!(doc.block_count(), 6, "the oldest steps were dropped");
}
