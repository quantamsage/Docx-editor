//! Pictures: recognizing uploaded image files and writing the DrawingML
//! markup Word uses for an inline picture.

use std::sync::Arc;

use quick_xml::escape::escape;
use serde::Serialize;

use crate::error::{DocxError, Result};
use crate::props::WNames;
use crate::xml::{Element, Node, XmlDocument};

pub const WP_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
pub const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const PIC_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
pub const IMAGE_REL_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

/// Largest picture size Word accepts: 22 inches, in EMUs.
pub const MAX_EMU: i64 = 22 * 914_400;
/// EMUs per CSS pixel (at 96 pixels per inch).
pub const EMU_PER_PIXEL: i64 = 9525;

/// Image formats Word displays and browsers can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
        }
    }
}

/// A picture uploaded for insertion. It becomes part of the document only
/// when an edit inserts it.
#[derive(Debug, Clone)]
pub struct StagedImage {
    pub id: String,
    pub format: ImageFormat,
    pub width_px: u32,
    pub height_px: u32,
    pub data: Arc<Vec<u8>>,
}

/// What the editor needs to know about a staged picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StagedImageInfo {
    pub id: String,
    pub format: ImageFormat,
    pub width_px: u32,
    pub height_px: u32,
}

impl StagedImage {
    pub fn info(&self) -> StagedImageInfo {
        StagedImageInfo { id: self.id.clone(), format: self.format, width_px: self.width_px, height_px: self.height_px }
    }
}

/// Recognizes an image by its content (never by its name) and reads its
/// size in pixels.
pub fn sniff(data: &[u8]) -> Result<(ImageFormat, u32, u32)> {
    let unsupported =
        || DocxError::InvalidEdit("only PNG, JPEG, GIF and BMP pictures can be inserted (the file is not one)".into());
    let le16 = |at: usize| data.get(at..at + 2).map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])));
    let be32 = |at: usize| data.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let le32 = |at: usize| data.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));

    let (format, size) = if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.get(12..16) == Some(b"IHDR") {
        (ImageFormat::Png, be32(16).zip(be32(20)))
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        (ImageFormat::Gif, le16(6).zip(le16(8)))
    } else if data.starts_with(b"BM") {
        let header = le32(14).unwrap_or(0);
        let size = if header == 12 {
            le16(18).zip(le16(20))
        } else {
            le32(18).zip(le32(22)).map(|(w, h)| (w.unsigned_abs(), h.unsigned_abs()))
        };
        (ImageFormat::Bmp, size)
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        (ImageFormat::Jpeg, jpeg_size(data))
    } else {
        return Err(unsupported());
    };
    match size {
        Some((w, h)) if w > 0 && h > 0 && w <= 65_535 && h <= 65_535 => Ok((format, w, h)),
        _ => Err(DocxError::InvalidEdit("the picture is damaged or has an unsupported size".into())),
    }
}

/// Width and height from a JPEG's start-of-frame segment.
fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return None;
        }
        let marker = data[at + 1];
        if marker == 0xFF {
            at += 1; // fill byte
            continue;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            at += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
        let is_sof = (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker);
        if is_sof {
            let h = u16::from_be_bytes([*data.get(at + 5)?, *data.get(at + 6)?]);
            let w = u16::from_be_bytes([*data.get(at + 7)?, *data.get(at + 8)?]);
            return Some((u32::from(w), u32::from(h)));
        }
        if length < 2 {
            return None;
        }
        at += 2 + length;
    }
    None
}

/// Checks a display size in EMUs.
pub fn validate_size(width_emu: i64, height_emu: i64) -> Result<()> {
    if !(1..=MAX_EMU).contains(&width_emu) || !(1..=MAX_EMU).contains(&height_emu) {
        return Err(DocxError::InvalidEdit("pictures must be between 1 pixel and 22 inches wide and high".into()));
    }
    Ok(())
}

/// What goes into a new picture's markup.
pub struct Picture<'a> {
    pub rel_id: &'a str,
    /// Unique drawing object id (`wp:docPr/@id`).
    pub id: u32,
    pub file_name: &'a str,
    pub description: &'a str,
    pub width_emu: i64,
    pub height_emu: i64,
}

/// `<w:r><w:drawing><wp:inline>…</wp:inline></w:drawing></w:r>` for a
/// picture, written the way Word writes it. DrawingML namespaces are
/// declared on the elements that use them, so the markup is valid wherever
/// it is inserted.
pub fn picture_run(w: &WNames, pic: &Picture<'_>) -> Element {
    let p = if w.prefix.is_empty() { String::new() } else { format!("{}:", w.prefix) };
    let r_ns = crate::read::R_NS;
    let (cx, cy, id) = (pic.width_emu, pic.height_emu, pic.id);
    let name = escape(pic.file_name);
    let descr = escape(pic.description);
    let rel = escape(pic.rel_id);
    let xml = format!(
        r#"<{p}r><{p}drawing><wp:inline xmlns:wp="{WP_NS}" distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:docPr id="{id}" name="Picture {id}" descr="{descr}"/><wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a="{A_NS}" noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic xmlns:a="{A_NS}"><a:graphicData uri="{PIC_NS}"><pic:pic xmlns:pic="{PIC_NS}"><pic:nvPicPr><pic:cNvPr id="0" name="{name}" descr="{descr}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip xmlns:r="{r_ns}" r:embed="{rel}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></{p}drawing></{p}r>"#
    );
    parse_in_namespace(w, &xml)
}

/// Parses markup written with the WordprocessingML prefix `w.prefix`.
pub fn parse_in_namespace(w: &WNames, xml: &str) -> Element {
    let wrapper = if w.prefix.is_empty() {
        format!(r#"<wrap xmlns="{}">{xml}</wrap>"#, w.ns)
    } else {
        format!(r#"<{p}:wrap xmlns:{p}="{}">{xml}</{p}:wrap>"#, w.ns, p = w.prefix)
    };
    let doc = XmlDocument::parse(wrapper.as_bytes()).expect("built-in markup is well formed");
    doc.root
        .children
        .into_iter()
        .find_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
        .expect("markup has an element")
}

/// Whether a run holds a picture (a drawing with an image fill).
pub fn run_picture_mut<'e>(run: &'e mut Element, w: &str) -> Option<&'e mut Element> {
    let drawing = run.elements_mut().find(|e| e.is(w, "drawing"))?;
    let container = drawing.elements_mut().find(|e| e.local_name() == "inline" || e.local_name() == "anchor")?;
    has_descendant(container, "blip").then_some(container)
}

fn has_descendant(el: &Element, local: &str) -> bool {
    el.elements().any(|c| c.local_name() == local || has_descendant(c, local))
}

/// Sets a picture's size: the drawing's extent and the picture shape's
/// transform. Other attributes (cropping, effects, wrapping) are kept.
pub fn set_picture_size(container: &mut Element, width_emu: i64, height_emu: i64) {
    let (cx, cy) = (width_emu.to_string(), height_emu.to_string());
    if let Some(extent) = container.elements_mut().find(|e| e.local_name() == "extent") {
        extent.set_attr("cx", None, &cx);
        extent.set_attr("cy", None, &cy);
    }
    if let Some(ext) = find_path_mut(container, &["graphic", "graphicData", "pic", "spPr", "xfrm", "ext"]) {
        ext.set_attr("cx", None, &cx);
        ext.set_attr("cy", None, &cy);
    }
}

fn find_path_mut<'e>(el: &'e mut Element, path: &[&str]) -> Option<&'e mut Element> {
    match path.split_first() {
        None => Some(el),
        Some((first, rest)) => find_path_mut(el.elements_mut().find(|e| e.local_name() == *first)?, rest),
    }
}

/// The largest `wp:docPr` id in a tree (drawing object ids must be unique
/// in a document).
pub fn max_drawing_id(el: &Element) -> u32 {
    el.elements()
        .map(|child| {
            let own = if child.local_name() == "docPr" {
                child.attr(None, "id").and_then(|v| v.parse().ok()).unwrap_or(0)
            } else {
                0
            };
            own.max(max_drawing_id(child))
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_formats_and_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&[0, 0, 1, 44, 0, 0, 0, 200]);
        assert_eq!(sniff(&png).unwrap(), (ImageFormat::Png, 300, 200));

        let gif = b"GIF89a\x40\x01\xf0\x00rest";
        assert_eq!(sniff(gif).unwrap(), (ImageFormat::Gif, 320, 240));

        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 12]);
        bmp.extend_from_slice(&40i32.to_le_bytes());
        bmp.extend_from_slice(&64i32.to_le_bytes());
        bmp.extend_from_slice(&(-32i32).to_le_bytes());
        assert_eq!(sniff(&bmp).unwrap(), (ImageFormat::Bmp, 64, 32));

        // SOI, an APP0 segment, then SOF0 with height 480 and width 640.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80, 0x03]);
        assert_eq!(sniff(&jpeg).unwrap(), (ImageFormat::Jpeg, 640, 480));

        for bad in [&b"<svg xmlns='http://www.w3.org/2000/svg'/>"[..], b"", b"\x89PNG\r\n\x1a\n", b"GIF89a\0\0\0\0"] {
            assert!(sniff(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn picture_markup_is_well_formed_and_sized() {
        let w = WNames { ns: crate::read::W_NS.into(), prefix: "w".into(), attr_prefix: "w".into() };
        let pic = Picture {
            rel_id: "rId7",
            id: 3,
            file_name: "image1.png",
            description: "A \"quoted\" <chart>",
            width_emu: 952_500,
            height_emu: 476_250,
        };
        let mut run = picture_run(&w, &pic);
        let mut out = String::new();
        run.write_to(&mut out);
        assert!(out.starts_with("<w:r><w:drawing><wp:inline"), "{out}");
        assert!(out.contains(r#"descr="A &quot;quoted&quot; &lt;chart&gt;""#), "{out}");
        assert!(out.contains(r#"r:embed="rId7""#));

        let container = run_picture_mut(&mut run, crate::read::W_NS).expect("a picture");
        assert_eq!(max_drawing_id(container), 3);
        set_picture_size(container, 100, 200);
        let mut out = String::new();
        run.write_to(&mut out);
        assert!(
            out.contains(r#"<wp:extent cx="100" cy="200"/>"#) && out.contains(r#"<a:ext cx="100" cy="200"/>"#),
            "{out}"
        );

        let doc =
            XmlDocument::parse(format!("<w:document xmlns:w=\"{}\">{out}</w:document>", w.ns).as_bytes()).unwrap();
        assert_eq!(max_drawing_id(&doc.root), 3);
    }
}
