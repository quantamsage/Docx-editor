//! An open document: the package, the parsed main part, and editing.

use crate::blank::blank_package;
use crate::error::{DocxError, Result};
use crate::model::{Alignment, DocumentModel};
use crate::opc::{
    ContentTypes, MAIN_CONTENT_TYPES, Relationship, parse_relationships, rel_type, rels_part_for, resolve_target,
};
use crate::package::{Limits, Package};
use crate::props::{PPR_ORDER, WNames, ensure_child, ensure_ppr, remove_child};
use crate::read::{ReadContext, W_NS, W_NS_STRICT, block_positions, read_document, read_styles};
use crate::xml::{Element, Node, XML_NS, XmlDocument};

/// Run children that hold only plain text; runs made of these are replaced
/// by text edits. Anything else (images, fields, footnote references, …)
/// keeps its run.
const PLAIN_RUN_CHILDREN: [&str; 8] =
    ["rPr", "t", "tab", "br", "cr", "noBreakHyphen", "softHyphen", "lastRenderedPageBreak"];

/// An open DOCX document.
#[derive(Debug, Clone)]
pub struct Document {
    package: Package,
    main_part: String,
    xml: XmlDocument,
    /// Set when the main part has been edited and must be re-serialized.
    modified: bool,
    w: WNames,
    rels: Vec<Relationship>,
    styles: Vec<crate::model::Style>,
    warnings: Vec<String>,
}

impl Document {
    /// Opens a document from the bytes of a `.docx` file.
    pub fn open(bytes: &[u8]) -> Result<Self> {
        Self::open_with_limits(bytes, Limits::default())
    }

    /// Opens a document with custom safety limits.
    pub fn open_with_limits(bytes: &[u8], limits: Limits) -> Result<Self> {
        Self::from_package(Package::read(bytes, limits)?)
    }

    /// Creates a new, empty document.
    pub fn new_blank() -> Self {
        Self::from_package(blank_package()).expect("the blank document template is valid")
    }

    fn from_package(package: Package) -> Result<Self> {
        let types_part = package
            .part("[Content_Types].xml")
            .ok_or_else(|| DocxError::NotWordDocument("it has no [Content_Types].xml part".into()))?;
        let content_types = ContentTypes::parse(&parse_part("[Content_Types].xml", &types_part.data)?);

        let root_rels_part = package
            .part("_rels/.rels")
            .ok_or_else(|| DocxError::NotWordDocument("it has no package relationships (_rels/.rels)".into()))?;
        let root_rels = parse_relationships(&parse_part("_rels/.rels", &root_rels_part.data)?);
        let main_part = root_rels
            .iter()
            .find(|r| r.is_type(rel_type::OFFICE_DOCUMENT) && !r.external)
            .and_then(|r| resolve_target("", &r.target))
            .ok_or_else(|| DocxError::NotWordDocument("it does not declare a main document".into()))?;
        let main = package
            .part(&main_part)
            .ok_or_else(|| DocxError::NotWordDocument(format!("its main document part '{main_part}' is missing")))?;
        let main_part = main.name.clone();

        match content_types.of(&main_part) {
            Some(ct) if MAIN_CONTENT_TYPES.contains(&ct) => {}
            Some(ct) => return Err(DocxError::NotWordDocument(format!("its main part has content type {ct}"))),
            None => return Err(DocxError::NotWordDocument("its main part has no content type".into())),
        }

        let xml = parse_part(&main_part, &main.data)?;
        let w_ns = match xml.root.namespace.as_deref() {
            Some(ns @ (W_NS | W_NS_STRICT)) if xml.root.local_name() == "document" => ns.to_string(),
            _ => return Err(DocxError::NotWordDocument("the main part is not a WordprocessingML document".into())),
        };
        if xml.root.child(&w_ns, "body").is_none() {
            return Err(DocxError::NotWordDocument("the document has no body".into()));
        }
        let prefix = xml.prefix_for(&w_ns).unwrap_or_else(|| "w".into());
        let attr_prefix = if prefix.is_empty() { "w".to_string() } else { prefix.clone() };
        let w = WNames { ns: w_ns, prefix, attr_prefix };

        let mut warnings = Vec::new();
        let rels = match package.part(&rels_part_for(&main_part)) {
            Some(part) => match XmlDocument::parse(&part.data) {
                Ok(doc) => parse_relationships(&doc),
                Err(e) => {
                    warnings.push(format!(
                        "The document's relationships could not be read ({e}); links and images may not display."
                    ));
                    Vec::new()
                }
            },
            None => Vec::new(),
        };

        let styles = rels
            .iter()
            .find(|r| r.is_type(rel_type::STYLES) && !r.external)
            .and_then(|r| resolve_target(&main_part, &r.target))
            .and_then(|name| package.part(&name))
            .map(|part| match XmlDocument::parse(&part.data) {
                Ok(doc) => read_styles(&doc, &w.ns),
                Err(e) => {
                    warnings.push(format!("The style definitions could not be read ({e}); they are kept unchanged."));
                    Vec::new()
                }
            })
            .unwrap_or_default();

        Ok(Self { package, main_part, xml, modified: false, w, rels, styles, warnings })
    }

    /// The typed read model of the document.
    pub fn model(&self) -> DocumentModel {
        let ctx = ReadContext { w: &self.w.ns, main_part: &self.main_part, rels: &self.rels };
        let mut model = read_document(&self.xml, &ctx);
        model.styles = self.styles.clone();
        model.warnings = self.warnings.clone();
        model
    }

    /// Serializes the document. If nothing was edited, every part is written
    /// back exactly as read.
    pub fn save(&self) -> Result<Vec<u8>> {
        if !self.modified {
            return self.package.write();
        }
        let main = self.xml.to_bytes();
        self.package.write_with(&[(self.main_part.as_str(), main.as_slice())])
    }

    /// Whether the document has unsaved edits since it was opened.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// The underlying package (all parts as read).
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// Name of the main document part (usually `word/document.xml`).
    pub fn main_part(&self) -> &str {
        &self.main_part
    }

    /// Number of top-level blocks (paragraphs, tables and other content).
    pub fn block_count(&self) -> usize {
        block_positions(self.body(), &self.w.ns).len()
    }

    // ----- Editing -----

    /// Replaces the plain text of paragraph `block`. Runs that contain only
    /// text are replaced by one run that keeps the formatting of the first of
    /// them; runs with other content (images, fields, …), hyperlinks and
    /// everything else in the paragraph are kept. `\n` becomes a line break
    /// and `\t` a tab.
    pub fn set_paragraph_text(&mut self, block: usize, text: &str) -> Result<()> {
        validate_text(text)?;
        let w = self.w.clone();
        let p = self.paragraph_mut(block)?;

        let plain: Vec<usize> = p
            .children
            .iter()
            .enumerate()
            .filter_map(|(i, n)| match n {
                Node::Element(e) if e.is(&w.ns, "r") && is_plain_run(e, &w.ns) => Some(i),
                _ => None,
            })
            .collect();
        let template_rpr = plain.first().and_then(|&i| match &p.children[i] {
            Node::Element(r) => r.child(&w.ns, "rPr").cloned(),
            _ => None,
        });
        let insert_at = match plain.first() {
            Some(&first) => first,
            None => {
                p.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(&w.ns, "pPr"))).map_or(0, |i| i + 1)
            }
        };
        for &i in plain.iter().rev() {
            p.children.remove(i);
        }
        if !text.is_empty() {
            p.children.insert(insert_at, Node::Element(build_run(&w, template_rpr, text)));
        }
        self.modified = true;
        Ok(())
    }

    /// Inserts a new paragraph so that it becomes block `at` (`at` may equal
    /// the block count to append).
    pub fn insert_paragraph(&mut self, at: usize, text: &str, style_id: Option<&str>) -> Result<()> {
        validate_text(text)?;
        let w = self.w.clone();
        let positions = block_positions(self.body(), &w.ns);
        if at > positions.len() {
            return Err(DocxError::InvalidEdit(format!("position {at} is past the end of the document")));
        }
        let mut p = w.element("p");
        if let Some(style) = style_id {
            validate_style_id(style)?;
            let ppr = ensure_ppr(&mut p, &w);
            w.set_attr(ensure_child(ppr, &w, PPR_ORDER, "pStyle"), "val", style);
        }
        if !text.is_empty() {
            p.children.push(Node::Element(build_run(&w, None, text)));
        }

        let body = self.body_mut();
        let index = match positions.get(at) {
            Some(&i) => i,
            // Append before the final section properties, if any.
            None => body
                .children
                .iter()
                .rposition(|n| matches!(n, Node::Element(e) if e.is(&w.ns, "sectPr")))
                .unwrap_or(body.children.len()),
        };
        body.children.insert(index, Node::Element(p));
        self.modified = true;
        Ok(())
    }

    /// Deletes block `block`. Paragraphs that end a section cannot be deleted
    /// (that would merge two sections' page layouts). If the body would be
    /// left empty, an empty paragraph is kept.
    pub fn delete_block(&mut self, block: usize) -> Result<()> {
        let w = self.w.clone();
        let positions = block_positions(self.body(), &w.ns);
        let &index =
            positions.get(block).ok_or_else(|| DocxError::InvalidEdit(format!("there is no block {block}")))?;
        if let Node::Element(el) = &self.body().children[index] {
            let ends_section =
                el.is(&w.ns, "p") && el.child(&w.ns, "pPr").and_then(|p| p.child(&w.ns, "sectPr")).is_some();
            if ends_section {
                return Err(DocxError::InvalidEdit("this paragraph ends a section and cannot be deleted".into()));
            }
        }
        let body = self.body_mut();
        body.children.remove(index);
        if positions.len() == 1 {
            body.children.insert(index, Node::Element(w.element("p")));
        }
        self.modified = true;
        Ok(())
    }

    /// Sets or clears (`None`) the paragraph style of paragraph `block`.
    pub fn set_paragraph_style(&mut self, block: usize, style_id: Option<&str>) -> Result<()> {
        if let Some(style) = style_id {
            validate_style_id(style)?;
        }
        let w = self.w.clone();
        let p = self.paragraph_mut(block)?;
        match style_id {
            Some(style) => {
                let ppr = ensure_ppr(p, &w);
                w.set_attr(ensure_child(ppr, &w, PPR_ORDER, "pStyle"), "val", style);
            }
            None => {
                if let Some(ppr) = p.child_mut(&w.ns, "pPr") {
                    remove_child(ppr, &w, "pStyle");
                }
            }
        }
        self.modified = true;
        Ok(())
    }

    /// Sets or clears (`None`) the alignment of paragraph `block`.
    pub fn set_paragraph_alignment(&mut self, block: usize, alignment: Option<Alignment>) -> Result<()> {
        let w = self.w.clone();
        let p = self.paragraph_mut(block)?;
        match alignment {
            Some(alignment) => {
                let ppr = ensure_ppr(p, &w);
                w.set_attr(ensure_child(ppr, &w, PPR_ORDER, "jc"), "val", alignment.to_ooxml());
            }
            None => {
                if let Some(ppr) = p.child_mut(&w.ns, "pPr") {
                    remove_child(ppr, &w, "jc");
                }
            }
        }
        self.modified = true;
        Ok(())
    }

    // ----- Helpers -----

    fn body(&self) -> &Element {
        self.xml.root.child(&self.w.ns, "body").expect("checked when the document was opened")
    }

    fn body_mut(&mut self) -> &mut Element {
        let ns = self.w.ns.clone();
        self.ensure_attr_prefix_declared();
        self.xml.root.child_mut(&ns, "body").expect("checked when the document was opened")
    }

    fn paragraph_mut(&mut self, block: usize) -> Result<&mut Element> {
        let ns = self.w.ns.clone();
        let positions = block_positions(self.body(), &ns);
        let &index =
            positions.get(block).ok_or_else(|| DocxError::InvalidEdit(format!("there is no block {block}")))?;
        match &mut self.body_mut().children[index] {
            Node::Element(el) if el.is(&ns, "p") => Ok(el),
            _ => Err(DocxError::InvalidEdit(format!("block {block} is not a paragraph"))),
        }
    }

    /// Documents that use WordprocessingML as the default namespace need a
    /// prefix declared for the attributes the engine writes.
    fn ensure_attr_prefix_declared(&mut self) {
        if self.w.prefix.is_empty() && self.xml.prefix_for(&self.w.ns).as_deref() != Some(self.w.attr_prefix.as_str()) {
            let declaration = format!("xmlns:{}", self.w.attr_prefix);
            let ns = self.w.ns.clone();
            self.xml.root.set_attr(&declaration, None, &ns);
        }
    }
}

fn parse_part(name: &str, data: &[u8]) -> Result<XmlDocument> {
    XmlDocument::parse(data).map_err(|e| DocxError::MalformedXml { part: name.to_string(), message: e.0 })
}

fn is_plain_run(run: &Element, w: &str) -> bool {
    run.children.iter().all(|n| match n {
        Node::Element(e) => {
            e.namespace.as_deref() == Some(w)
                && PLAIN_RUN_CHILDREN.contains(&e.local_name())
                && !(e.local_name() == "br" && e.attr(Some(w), "type").is_some_and(|t| t != "textWrapping"))
        }
        Node::Text(t) => t.trim().is_empty(),
        _ => true,
    })
}

/// Builds `<w:r>[rPr]<w:t>…</w:t><w:tab/><w:br/>…</w:r>` for `text`.
fn build_run(w: &WNames, rpr: Option<Element>, text: &str) -> Element {
    let mut run = w.element("r");
    if let Some(rpr) = rpr {
        run.children.push(Node::Element(rpr));
    }
    let mut segment = String::new();
    let flush = |segment: &mut String, run: &mut Element| {
        if segment.is_empty() {
            return;
        }
        let mut t = w.element("t");
        if segment.starts_with(char::is_whitespace) || segment.ends_with(char::is_whitespace) {
            t.set_attr("xml:space", Some(XML_NS), "preserve");
        }
        t.set_text(segment);
        run.children.push(Node::Element(t));
        segment.clear();
    };
    for ch in text.chars() {
        match ch {
            '\t' => {
                flush(&mut segment, &mut run);
                run.children.push(Node::Element(w.element("tab")));
            }
            '\n' => {
                flush(&mut segment, &mut run);
                run.children.push(Node::Element(w.element("br")));
            }
            '\r' => {}
            c => segment.push(c),
        }
    }
    flush(&mut segment, &mut run);
    run
}

/// Rejects characters that XML 1.0 cannot represent.
fn validate_text(text: &str) -> Result<()> {
    let invalid =
        text.chars().find(|&c| (c < ' ' && !matches!(c, '\t' | '\n' | '\r')) || matches!(c, '\u{FFFE}' | '\u{FFFF}'));
    match invalid {
        Some(c) => {
            Err(DocxError::InvalidEdit(format!("the text contains an unsupported character (U+{:04X})", c as u32)))
        }
        None => Ok(()),
    }
}

fn validate_style_id(style: &str) -> Result<()> {
    if style.is_empty() || style.len() > 253 || style.chars().any(char::is_control) {
        return Err(DocxError::InvalidEdit("invalid style identifier".into()));
    }
    Ok(())
}
