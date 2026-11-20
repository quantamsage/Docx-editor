//! An open document: the package, the parsed main part, and editing.

use crate::blank::blank_package;
use crate::edit::{Edit, InlineRef, Piece, Splice};
use crate::error::{DocxError, Result};
use crate::model::{Alignment, DocumentModel};
use crate::opc::{
    ContentTypes, MAIN_CONTENT_TYPES, Relationship, parse_relationships, rel_type, rels_part_for, resolve_target,
};
use crate::package::{Limits, Package};
use crate::props::{PPR_ORDER, WNames, ensure_child, ensure_ppr, remove_child};
use crate::read::{ReadContext, W_NS, W_NS_STRICT, block_positions, is_inline, read_document, read_styles};
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

    /// The bytes of an image part the main document references as an image
    /// (`word/media/…`), for display. Other parts are not returned.
    pub fn image_part(&self, name: &str) -> Option<&[u8]> {
        let referenced = self.rels.iter().any(|r| {
            r.is_type(rel_type::IMAGE)
                && !r.external
                && resolve_target(&self.main_part, &r.target).is_some_and(|target| target.eq_ignore_ascii_case(name))
        });
        if !referenced {
            return None;
        }
        self.package.part(name).map(|p| p.data.as_slice())
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

    /// Applies edits atomically: if any edit fails, the document is left
    /// exactly as it was and the error is returned.
    pub fn apply(&mut self, edits: &[Edit]) -> Result<()> {
        let saved = (self.xml.clone(), self.modified);
        for (i, edit) in edits.iter().enumerate() {
            let result = match edit {
                Edit::Splice(s) => self.splice(s),
                Edit::InsertParagraph { at, text, style_id } => self.insert_paragraph(*at, text, style_id.as_deref()),
                Edit::DeleteBlock { block } => self.delete_block(*block),
                Edit::SetParagraphStyle { block, style_id } => self.set_paragraph_style(*block, style_id.as_deref()),
                Edit::SetParagraphAlignment { block, alignment } => self.set_paragraph_alignment(*block, *alignment),
            };
            if let Err(e) = result {
                (self.xml, self.modified) = saved;
                return Err(match e {
                    DocxError::InvalidEdit(msg) if edits.len() > 1 => {
                        DocxError::InvalidEdit(format!("edit {} of {}: {msg}", i + 1, edits.len()))
                    }
                    other => other,
                });
            }
        }
        Ok(())
    }

    /// Replaces paragraphs `first..=last` with new content; see [`Splice`].
    /// On error the document is unchanged.
    pub fn splice(&mut self, splice: &Splice) -> Result<()> {
        let w = self.w.clone();
        let Splice { first, last, content } = splice;
        let (first, last) = (*first, *last);
        let positions = block_positions(self.body(), &w.ns);
        if first > last || last >= positions.len() {
            return Err(DocxError::InvalidEdit(format!(
                "paragraphs {first}..{last} are not in the document ({} blocks)",
                positions.len()
            )));
        }

        // The paragraphs involved, and the virtual sequence of their content.
        let body = self.body();
        let mut paragraphs: Vec<&Element> = Vec::with_capacity(last - first + 1);
        for (block, &position) in positions.iter().enumerate().take(last + 1).skip(first) {
            match &body.children[position] {
                Node::Element(el) if el.is(&w.ns, "p") => paragraphs.push(el),
                _ => return Err(DocxError::InvalidEdit(format!("block {block} is not a paragraph"))),
            }
        }
        for (offset, p) in paragraphs[..paragraphs.len() - 1].iter().enumerate() {
            if section_properties(p, &w.ns).is_some() {
                return Err(DocxError::InvalidEdit(format!(
                    "paragraph {} ends a section; the edit would remove the section break",
                    first + offset
                )));
            }
        }

        // (block, inline index or None for markers, node)
        let mut sequence: Vec<(usize, Option<usize>, &Node)> = Vec::new();
        let mut inline_counts = Vec::with_capacity(paragraphs.len());
        for (offset, p) in paragraphs.iter().enumerate() {
            let mut count = 0;
            for node in &p.children {
                match node {
                    Node::Element(el) if el.is(&w.ns, "pPr") => {}
                    Node::Element(el) if is_inline(el, &w.ns) => {
                        sequence.push((first + offset, Some(count), node));
                        count += 1;
                    }
                    other => sequence.push((first + offset, None, other)),
                }
            }
            inline_counts.push(count);
        }
        let inline_at = |r: InlineRef| -> Result<&Element> {
            if r.block < first || r.block > last || r.inline >= inline_counts[r.block - first] {
                return Err(DocxError::InvalidEdit(format!(
                    "inline {} of block {} is not part of the edited paragraphs",
                    r.inline, r.block
                )));
            }
            let node = sequence
                .iter()
                .find(|(b, i, _)| *b == r.block && *i == Some(r.inline))
                .map(|(_, _, n)| *n)
                .expect("counted above");
            match node {
                Node::Element(el) => Ok(el),
                _ => unreachable!("inlines are elements"),
            }
        };

        // Validate the pieces.
        let mut kept: Vec<InlineRef> = Vec::new();
        for piece in content {
            match piece {
                Piece::Keep { block, inline } => {
                    let r = InlineRef { block: *block, inline: *inline };
                    inline_at(r)?;
                    if kept.last().is_some_and(|prev| *prev >= r) {
                        return Err(DocxError::InvalidEdit(
                            "kept content must be listed once, in document order".into(),
                        ));
                    }
                    kept.push(r);
                }
                Piece::Text { text, like } => {
                    validate_text(text)?;
                    if let Some(r) = like
                        && !inline_at(*r)?.is(&w.ns, "r")
                    {
                        return Err(DocxError::InvalidEdit(format!(
                            "inline {} of block {} is not a run, so its formatting cannot be copied",
                            r.inline, r.block
                        )));
                    }
                }
                Piece::Break => {}
            }
        }

        // Build the new paragraphs' content by walking the sequence: kept
        // inlines and markers stay in order; text and breaks are flushed at
        // the first removed inline (where the edit happened) or before the
        // next kept inline.
        let mut outputs: Vec<Vec<Node>> = vec![Vec::new()];
        let mut cursor = 0;
        let flush_until = |end: usize, cursor: &mut usize, outputs: &mut Vec<Vec<Node>>| -> Result<()> {
            while *cursor < end {
                match &content[*cursor] {
                    Piece::Text { text, like } if !text.is_empty() => {
                        let rpr = match like {
                            Some(r) => inline_at(*r)?.child(&w.ns, "rPr").cloned(),
                            None => None,
                        };
                        outputs.last_mut().expect("never empty").push(Node::Element(build_run(&w, rpr, text)));
                    }
                    Piece::Break => outputs.push(Vec::new()),
                    _ => {}
                }
                *cursor += 1;
            }
            Ok(())
        };
        let next_keep =
            |from: usize| content[from..].iter().position(|p| matches!(p, Piece::Keep { .. })).map(|i| from + i);
        for &(block, inline, node) in &sequence {
            match inline {
                None => outputs.last_mut().expect("never empty").push(node.clone()),
                Some(i) => {
                    let keep_at = content
                        .iter()
                        .position(|p| matches!(p, Piece::Keep { block: b, inline: n } if *b == block && *n == i));
                    match keep_at {
                        Some(at) => {
                            flush_until(at, &mut cursor, &mut outputs)?;
                            outputs.last_mut().expect("never empty").push(node.clone());
                            cursor = at + 1;
                        }
                        None => {
                            let end = next_keep(cursor).unwrap_or(content.len());
                            flush_until(end, &mut cursor, &mut outputs)?;
                        }
                    }
                }
            }
        }
        flush_until(content.len(), &mut cursor, &mut outputs)?;

        // Paragraph properties: the first paragraph's, without its section
        // break; the section break of the last paragraph goes on the last
        // new paragraph.
        let first_p = paragraphs[0];
        let mut base_ppr = first_p.child(&w.ns, "pPr").cloned();
        let section = section_properties(paragraphs[paragraphs.len() - 1], &w.ns).cloned();
        if let Some(ppr) = base_ppr.as_mut() {
            remove_child(ppr, &w, "sectPr");
        }
        let count = outputs.len();
        let mut new_paragraphs = Vec::with_capacity(count);
        for (index, nodes) in outputs.into_iter().enumerate() {
            let mut p = if index == 0 {
                let mut p = first_p.clone();
                p.children.clear();
                p
            } else {
                w.element("p")
            };
            let mut ppr = base_ppr.clone();
            if index == count - 1
                && let Some(sect) = &section
            {
                let ppr = ppr.get_or_insert_with(|| w.element("pPr"));
                *ensure_child(ppr, &w, PPR_ORDER, "sectPr") = sect.clone();
            }
            if let Some(ppr) = ppr.filter(|ppr| !ppr.children.is_empty() || index == 0) {
                p.children.push(Node::Element(ppr));
            }
            p.children.extend(nodes);
            new_paragraphs.push(Node::Element(p));
        }

        let body = self.body_mut();
        for block in (first..=last).rev() {
            body.children.remove(positions[block]);
        }
        let at = positions[first];
        body.children.splice(at..at, new_paragraphs);
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

/// The section properties a paragraph carries (`w:pPr/w:sectPr`), if any.
fn section_properties<'e>(p: &'e Element, w: &str) -> Option<&'e Element> {
    p.child(w, "pPr").and_then(|ppr| ppr.child(w, "sectPr"))
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
