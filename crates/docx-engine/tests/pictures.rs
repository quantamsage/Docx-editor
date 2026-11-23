//! Pictures: staging an upload, inserting it with a splice (part,
//! relationship, content type and drawing markup), reusing it, resizing,
//! deleting it again and pictures in headers.

mod common;

use common::*;
use docx_engine::model::{Block, Inline, RunContent};
use docx_engine::{Container, Document, DocumentModel, Edit, Piece, Splice};

/// A 2×1 GIF.
const GIF: &[u8] = b"GIF89a\x02\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x00\x00\x00\x00\x00,\x00\x00\x00\x00\x02\x00\x01\x00\x00\x02\x02\x04\x0a\x00;";

fn image(id: &str, w: i64, h: i64) -> Piece {
    Piece::Image { image: id.into(), width_emu: w, height_emu: h, description: Some("Company logo".into()) }
}

fn keep(block: usize, inline: usize) -> Piece {
    Piece::Keep { block, inline }
}

fn images(blocks: &[Block]) -> Vec<docx_engine::model::Image> {
    let mut out = Vec::new();
    for block in blocks {
        if let Block::Paragraph(p) = block {
            for inline in &p.inlines {
                if let Inline::Run(r) = inline {
                    for c in &r.content {
                        if let RunContent::Image(img) = c {
                            out.push(img.clone());
                        }
                    }
                }
            }
        }
    }
    out
}

fn round_trip(doc: &Document) -> DocumentModel {
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    Document::open(&saved).unwrap().model()
}

#[test]
fn staging_recognizes_pictures_by_content() {
    let mut doc = Document::new_blank();
    let info = doc.stage_image(GIF.to_vec()).unwrap();
    assert_eq!((info.width_px, info.height_px, info.id.as_str()), (2, 1, "img1"));
    assert_eq!(serde_json::to_value(&info).unwrap()["format"], "gif");
    assert_eq!(doc.stage_image(PNG.to_vec()).unwrap().id, "img2");
    let err = doc.stage_image(b"<svg/>".to_vec()).unwrap_err();
    assert!(err.to_string().contains("PNG, JPEG, GIF and BMP"), "{err}");
    assert!(!doc.is_modified(), "staging alone does not change the document");
}

#[test]
fn inserting_a_picture_adds_part_relationship_and_content_type() {
    let mut doc = Document::new_blank();
    let id = doc.stage_image(GIF.to_vec()).unwrap().id;
    doc.splice(&Splice {
        container: Container::body(),
        first: 0,
        last: 0,
        content: vec![image(&id, 952_500, 476_250)],
    })
    .unwrap();
    let saved = doc.save().unwrap();
    let model = round_trip(&doc);
    let pics = images(&model.blocks);
    assert_eq!(pics.len(), 1);
    assert_eq!(pics[0].part.as_deref(), Some("word/media/image1.gif"));
    assert_eq!((pics[0].width_emu, pics[0].height_emu), (Some(952_500), Some(476_250)));
    assert_eq!(pics[0].description.as_deref(), Some("Company logo"));
    assert_eq!(model.stats.images, 1);

    let parts = read_parts(&saved);
    assert_eq!(parts.iter().find(|(n, _, _)| n == "word/media/image1.gif").unwrap().1, GIF);
    let types = part_text(&saved, "[Content_Types].xml");
    assert!(types.contains(r#"<Default Extension="gif" ContentType="image/gif"/><Override"#), "{types}");
    let rels = part_text(&saved, "word/_rels/document.xml.rels");
    assert!(rels.contains(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.gif""#), "{rels}");
    let xml = part_text(&saved, "word/document.xml");
    assert!(xml.contains(r#"<wp:docPr id="1" name="Picture 1" descr="Company logo"/>"#), "{xml}");

    let reopened = Document::open(&saved).unwrap();
    assert_eq!(reopened.image_part("word/media/image1.gif"), Some(GIF));
    assert_eq!(doc.image_part("word/media/image1.gif"), Some(GIF), "shown before saving");
}

#[test]
fn pictures_get_unique_names_and_ids_and_are_reused() {
    let bytes = representative_docx();
    let mut doc = Document::open(&bytes).unwrap();
    let id = doc.stage_image(PNG.to_vec()).unwrap().id;
    // Two copies of the same picture around "Figure 1" (block 3).
    doc.splice(&Splice {
        container: Container::body(),
        first: 3,
        last: 3,
        content: vec![keep(3, 0), image(&id, 100, 100), keep(3, 1), image(&id, 200, 200)],
    })
    .unwrap();
    let saved = doc.save().unwrap();
    let pics = images(&round_trip(&doc).blocks);
    assert_eq!(pics.len(), 3);
    assert_eq!(pics[1].part.as_deref(), Some("word/media/image2.png"), "image1.png already exists");
    assert_eq!(pics[1].rel_id, pics[2].rel_id, "one part and relationship for both copies");
    assert_eq!(read_parts(&saved).iter().filter(|(n, _, _)| n.starts_with("word/media/")).count(), 2);
    let xml = part_text(&saved, "word/document.xml");
    for id in ["1", "2", "3"] {
        assert_eq!(xml.matches(&format!("<wp:docPr id=\"{id}\"")).count(), 1, "drawing ids are unique");
    }
    // PNG already had a content type; the original picture is untouched.
    assert_eq!(
        changed_parts(&bytes, &saved),
        ["word/_rels/document.xml.rels", "word/document.xml", "word/media/image2.png"]
    );
}

#[test]
fn deleted_pictures_are_left_out_when_saving() {
    let bytes = representative_docx();
    let mut doc = Document::open(&bytes).unwrap();
    let id = doc.stage_image(GIF.to_vec()).unwrap().id;
    doc.splice(&Splice {
        container: Container::body(),
        first: 0,
        last: 0,
        content: vec![keep(0, 0), image(&id, 9525, 9525)],
    })
    .unwrap();
    doc.splice(&Splice { container: Container::body(), first: 0, last: 0, content: vec![keep(0, 0)] }).unwrap();
    let saved = doc.save().unwrap();
    assert_valid_docx(&saved);
    assert!(!read_parts(&saved).iter().any(|(n, _, _)| n.ends_with(".gif")));
    assert!(!part_text(&saved, "word/_rels/document.xml.rels").contains("image1.gif"));
    assert_eq!(part_text(&saved, "word/_rels/document.xml.rels"), DOCUMENT_RELS);
}

#[test]
fn resizing_changes_only_the_size() {
    let mut doc = Document::open(&representative_docx()).unwrap();
    doc.apply(&[Edit::ResizeImage {
        container: Container::body(),
        block: 3,
        inline: 0,
        width_emu: 1_828_800,
        height_emu: 914_400,
    }])
    .unwrap();
    let pics = images(&round_trip(&doc).blocks);
    assert_eq!((pics[0].width_emu, pics[0].height_emu), (Some(1_828_800), Some(914_400)));
    let xml = part_text(&doc.save().unwrap(), "word/document.xml");
    assert!(
        xml.contains(r#"<wp:extent cx="1828800" cy="914400"/><wp:docPr id="1" name="Picture 1" descr="Sales chart"/>"#),
        "{xml}"
    );
    assert!(xml.contains(r#"<a:ext cx="1828800" cy="914400"/>"#));

    for (block, inline, w, h, expected) in [
        (3, 1, 10, 10, "not a picture"),
        (0, 0, 10, 10, "not a picture"),
        (3, 0, 0, 10, "between"),
        (3, 0, 10, i64::MAX, "between"),
    ] {
        let err = doc
            .apply(&[Edit::ResizeImage { container: Container::body(), block, inline, width_emu: w, height_emu: h }])
            .unwrap_err();
        assert!(err.to_string().contains(expected), "{err}");
    }
}

#[test]
fn unknown_pictures_are_refused_without_changes() {
    let mut doc = Document::new_blank();
    let err = doc
        .splice(&Splice { container: Container::body(), first: 0, last: 0, content: vec![image("img9", 10, 10)] })
        .unwrap_err();
    assert!(err.to_string().contains("no longer available"), "{err}");
    assert!(!doc.is_modified());
}

#[test]
fn pictures_in_headers_use_the_headers_relationships() {
    let bytes = representative_docx();
    let mut doc = Document::open(&bytes).unwrap();
    let id = doc.stage_image(GIF.to_vec()).unwrap().id;
    doc.splice(&Splice {
        container: Container::part("word/header1.xml"),
        first: 0,
        last: 0,
        content: vec![image(&id, 9525, 9525), keep(0, 0)],
    })
    .unwrap();
    let saved = doc.save().unwrap();
    assert_eq!(
        changed_parts(&bytes, &saved),
        ["[Content_Types].xml", "word/_rels/header1.xml.rels", "word/header1.xml", "word/media/image1.gif"]
    );
    let rels = part_text(&saved, "word/_rels/header1.xml.rels");
    assert!(rels.contains(r#"Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.gif""#), "{rels}");
    let reopened = Document::open(&saved).unwrap();
    let header = reopened.model().header.unwrap();
    assert_eq!(images(&header.blocks)[0].part.as_deref(), Some("word/media/image1.gif"));
    assert_eq!(reopened.image_part("word/media/image1.gif"), Some(GIF), "header pictures can be displayed");
    assert_valid_docx(&saved);
}
