//! A synthetic but representative DOCX used by the engine tests. It packs
//! in the constructs that most often get lost by editors: bookmarks, a
//! hyperlink, an inline image, a table with merged cells, a numbered list, a
//! content control, a field, a comment reference, alternate content, an
//! unknown extension element, a page break, a header and footer, numbering,
//! comments, custom XML and an unusual section layout.

#![allow(dead_code)]

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

pub const DOCUMENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:x="urn:example:custom" mc:Ignorable="w14 x"><w:body><w:p w14:paraId="1A2B3C4D" w:rsidR="00A1"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Quarterly Report</w:t></w:r></w:p><w:p w14:paraId="2B3C4D5E" w:rsidR="00A2" w:rsidRDefault="00A2"><w:bookmarkStart w:id="0" w:name="summary"/><w:r><w:t xml:space="preserve">Revenue grew </w:t></w:r><w:r><w:rPr><w:b/><w:color w:val="c00000"/></w:rPr><w:t>12%</w:t></w:r><w:proofErr w:type="spellStart"/><w:r><w:rPr><w:i/><w:u w:val="single"/><w:strike/><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia"/><w:sz w:val="28"/><w:highlight w:val="yellow"/></w:rPr><w:t xml:space="preserve"> this quarter</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:p><w:r><w:t xml:space="preserve">Visit </w:t></w:r><w:hyperlink r:id="rId5" w:history="1"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>Example site</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> or see </w:t></w:r><w:hyperlink w:anchor="summary"><w:r><w:t>the summary</w:t></w:r></w:hyperlink></w:p><w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="914400" cy="457200"/><wp:docPr id="1" name="Picture 1" descr="Sales chart"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="chart.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId6"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r><w:r><w:t xml:space="preserve"> Figure 1</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="4508"/><w:gridCol w:w="4508"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="9016" w:type="dxa"/><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>Merged header</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:p><w:r><w:t>C</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>First item</w:t></w:r></w:p><w:sdt><w:sdtPr><w:alias w:val="Client"/><w:id w:val="-1"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Content control text</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:commentReference w:id="0"/></w:r></w:p><w:p><w:r><mc:AlternateContent><mc:Choice Requires="w14"><w14:shadow/></mc:Choice><mc:Fallback/></mc:AlternateContent><w:t>Alt</w:t></w:r><x:custom x:flag="1">Extension text</x:custom></w:p><w:p><w:r><w:br w:type="page"/></w:r></w:p><w:p><w:pPr><w:spacing w:before="120" w:after="240" w:line="360" w:lineRule="auto"/><w:ind w:left="720" w:hanging="360"/><w:jc w:val="both"/></w:pPr><w:r><w:t xml:space="preserve">Terms &amp; conditions &lt;apply&gt; to	all</w:t><w:tab/><w:t>users</w:t><w:br/><w:t>Second line</w:t></w:r></w:p><w:sectPr w:rsidR="00A1"><w:headerReference w:type="default" r:id="rId7"/><w:footerReference w:type="default" r:id="rId8"/><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/><w:pgMar w:top="1080" w:right="1440" w:bottom="1080" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/><w:titlePg/><w:docGrid w:linePitch="360"/></w:sectPr></w:body></w:document>"#;

pub const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/></w:style><w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/></w:style></w:styles>"#;

pub const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/report?q=1&amp;r=2" TargetMode="External"/><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/></Relationships>"#;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const NUMBERING_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#;

const COMMENTS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:comment w:id="0" w:author="Reviewer"><w:p><w:r><w:t>Check this</w:t></w:r></w:p></w:comment></w:comments>"#;

const HEADER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Company Confidential</w:t></w:r></w:p></w:hdr>"#;

const FOOTER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>"#;

const CUSTOM_XML: &str =
    r#"<?xml version="1.0" encoding="UTF-8"?><data xmlns="urn:example:data"><client>ACME</client></data>"#;

/// A 1×1 PNG.
pub const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D,
    0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// The parts of the representative document, in archive order.
pub fn representative_parts() -> Vec<(String, Vec<u8>, CompressionMethod)> {
    vec![
        ("[Content_Types].xml".into(), CONTENT_TYPES.into(), CompressionMethod::Deflated),
        ("_rels/.rels".into(), ROOT_RELS.into(), CompressionMethod::Deflated),
        ("word/document.xml".into(), DOCUMENT_XML.into(), CompressionMethod::Deflated),
        ("word/_rels/document.xml.rels".into(), DOCUMENT_RELS.into(), CompressionMethod::Deflated),
        ("word/styles.xml".into(), STYLES_XML.into(), CompressionMethod::Deflated),
        ("word/numbering.xml".into(), NUMBERING_XML.into(), CompressionMethod::Deflated),
        ("word/comments.xml".into(), COMMENTS_XML.into(), CompressionMethod::Deflated),
        ("word/header1.xml".into(), HEADER_XML.into(), CompressionMethod::Deflated),
        ("word/footer1.xml".into(), FOOTER_XML.into(), CompressionMethod::Deflated),
        ("word/media/image1.png".into(), PNG.to_vec(), CompressionMethod::Stored),
        ("customXml/item1.xml".into(), CUSTOM_XML.into(), CompressionMethod::Deflated),
    ]
}

pub fn build_docx(parts: &[(String, Vec<u8>, CompressionMethod)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data, method) in parts {
        writer.start_file(name.as_str(), SimpleFileOptions::default().compression_method(*method)).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

pub fn representative_docx() -> Vec<u8> {
    build_docx(&representative_parts())
}

/// Builds a document whose parts are replaced or removed (`None`).
pub fn docx_with(changes: &[(&str, Option<&str>)]) -> Vec<u8> {
    let mut parts = representative_parts();
    for (name, replacement) in changes {
        match replacement {
            Some(data) => match parts.iter_mut().find(|(n, _, _)| n == name) {
                Some(part) => part.1 = data.as_bytes().to_vec(),
                None => parts.push((name.to_string(), data.as_bytes().to_vec(), CompressionMethod::Deflated)),
            },
            None => parts.retain(|(n, _, _)| n != name),
        }
    }
    build_docx(&parts)
}

/// All parts of an archive: (name, bytes, stored-or-deflated).
pub fn read_parts(bytes: &[u8]) -> Vec<(String, Vec<u8>, CompressionMethod)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut data).unwrap();
            (file.name().unwrap().into_owned(), data, file.compression())
        })
        .collect()
}

pub fn part_text(bytes: &[u8], name: &str) -> String {
    let (_, data, _) = read_parts(bytes).into_iter().find(|(n, _, _)| n == name).expect("part exists");
    String::from_utf8(data).unwrap()
}

/// Checks a saved document: it reopens, and its main document, headers and
/// footers have no schema-order or structure problems Word would reject.
pub fn assert_valid_docx(bytes: &[u8]) {
    docx_engine::Document::open(bytes).expect("saved document reopens");
    for (name, data, _) in read_parts(bytes) {
        let is_word_xml = name.starts_with("word/")
            && name.ends_with(".xml")
            && (name.contains("document") || name.contains("header") || name.contains("footer"));
        if is_word_xml {
            let doc = docx_engine::xml::XmlDocument::parse(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut problems = docx_engine::check::problems(&doc, W);
            // The representative document deliberately has one run whose
            // properties are out of order; edits must not add new problems.
            if name == "word/document.xml" {
                let original = docx_engine::xml::XmlDocument::parse(DOCUMENT_XML.as_bytes()).unwrap();
                for known in docx_engine::check::problems(&original, W) {
                    if let Some(i) = problems.iter().position(|p| *p == known) {
                        problems.remove(i);
                    }
                }
            }
            assert!(problems.is_empty(), "{name}: {problems:?}");
        }
    }
}

/// Parts whose bytes differ between two archives (and parts only in one).
pub fn changed_parts(before: &[u8], after: &[u8]) -> Vec<String> {
    let before = read_parts(before);
    let after = read_parts(after);
    let mut changed: Vec<String> = after
        .iter()
        .filter(|(name, data, _)| before.iter().find(|(n, _, _)| n == name).is_none_or(|(_, d, _)| d != data))
        .map(|(name, _, _)| name.clone())
        .collect();
    changed.extend(
        before.iter().filter(|(n, _, _)| !after.iter().any(|(m, _, _)| m == n)).map(|(n, _, _)| format!("-{n}")),
    );
    changed.sort();
    changed
}
