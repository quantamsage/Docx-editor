//! An open document: the package, the parsed main part, and editing.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::blank::{BLANK_STYLES, blank_package};
use crate::edit::{CellRef, Container, Edit, HeaderFooterKind, InlineRef, Piece, Splice};
use crate::error::{DocxError, Result};
use crate::format::{
    ListKind, ParagraphFormat, SectionFormat, apply_paragraph_format, apply_run_format, apply_section_format,
};
use crate::media::{self, IMAGE_REL_TYPE, StagedImage, StagedImageInfo};
use crate::model::{Alignment, DocumentModel, HeaderFooter, Stats};
use crate::numbering::{self, ListDefinition, NUMBERING_CONTENT_TYPE, NUMBERING_REL_TYPE};
use crate::opc::{
    CONTENT_TYPES_NS, ContentTypes, MAIN_CONTENT_TYPES, PACKAGE_RELS_NS, Relationship, parse_relationships, rel_type,
    rels_part_for, resolve_target,
};
use crate::package::{Limits, Package};
use crate::props::{PPR_ORDER, WNames, ensure_child, ensure_ppr, insert_reference, remove_child};
use crate::read::{
    R_NS, R_NS_STRICT, ReadContext, W_NS, W_NS_STRICT, block_positions, is_inline, read_blocks, read_document,
    read_styles,
};
use crate::table;
use crate::xml::{Element, Node, XML_NS, XmlDocument};

/// Run children that hold only plain text; runs made of these are replaced
/// by text edits. Anything else (images, fields, footnote references, …)
/// keeps its run.
const PLAIN_RUN_CHILDREN: [&str; 8] =
    ["rPr", "t", "tab", "br", "cr", "noBreakHyphen", "softHyphen", "lastRenderedPageBreak"];

const HEADER_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
const FOOTER_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
const HEADER_REL_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
const FOOTER_REL_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";

/// Most bytes of pictures waiting to be inserted (uploads not yet used).
pub const MAX_STAGED_BYTES: usize = 256 << 20;

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
    /// Secondary XML parts changed by edits (styles, numbering, headers,
    /// footers, relationships, content types), parsed on first change.
    /// Written on save in place of the originals; names not in the package
    /// are new parts.
    edited: BTreeMap<String, XmlDocument>,
    /// Binary parts added by edits (pictures).
    added: BTreeMap<String, Arc<Vec<u8>>>,
    /// Pictures uploaded for insertion. They only become part of the
    /// document when a splice inserts them.
    staged: BTreeMap<String, StagedImage>,
    next_image: u32,
}

/// The state edits change, for rolling back a failed batch.
struct Checkpoint {
    xml: XmlDocument,
    modified: bool,
    edited: BTreeMap<String, XmlDocument>,
    added: BTreeMap<String, Arc<Vec<u8>>>,
    rels: Vec<Relationship>,
    styles: Vec<crate::model::Style>,
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

        Ok(Self {
            package,
            main_part,
            xml,
            modified: false,
            w,
            rels,
            styles,
            warnings,
            edited: BTreeMap::new(),
            added: BTreeMap::new(),
            staged: BTreeMap::new(),
            next_image: 1,
        })
    }

    /// The typed read model of the document.
    pub fn model(&self) -> DocumentModel {
        let ctx = ReadContext { w: &self.w.ns, main_part: &self.main_part, rels: &self.rels };
        let mut model = read_document(&self.xml, &ctx);
        model.styles = self.styles.clone();
        model.warnings = self.warnings.clone();
        model.lists = self.lists();
        model.header = self.header_footer_content(HeaderFooterKind::Header);
        model.footer = self.header_footer_content(HeaderFooterKind::Footer);
        model
    }

    /// The document's list definitions (empty if it has no numbering part).
    fn lists(&self) -> Vec<ListDefinition> {
        let Some(name) = self.related_part(rel_type::NUMBERING) else { return Vec::new() };
        self.part_xml(&name).map(|doc| numbering::read(&doc, &self.w.ns)).unwrap_or_default()
    }

    /// The part the main document relates to with the given type, if any.
    fn related_part(&self, kind: &str) -> Option<String> {
        self.rels
            .iter()
            .find(|r| r.is_type(kind) && !r.external)
            .and_then(|r| resolve_target(&self.main_part, &r.target))
    }

    /// Serializes the document. If nothing was edited, every part is written
    /// back exactly as read; otherwise only edited parts are re-serialized.
    /// Pictures that were inserted and later deleted again are left out,
    /// with their relationships.
    ///
    /// An edited document is checked before it is returned: the archive is
    /// read back, it must open as a Word document, and every rewritten XML
    /// part must parse. If not, an error is returned instead of a file Word
    /// might not open.
    pub fn save(&self) -> Result<Vec<u8>> {
        if !self.is_modified() {
            return self.package.write();
        }
        let mut xml_parts: BTreeMap<String, Vec<u8>> =
            self.edited.iter().map(|(name, doc)| (name.clone(), doc.to_bytes())).collect();
        if self.modified {
            xml_parts.insert(self.main_part.clone(), self.xml.to_bytes());
        }

        let mut unused: Vec<&str> = Vec::new();
        for picture in self.added.keys() {
            let mut used = false;
            let mut references: Vec<(String, String)> = Vec::new();
            for (rels_name, rels_doc) in self.edited.iter().filter(|(n, _)| n.ends_with(".rels")) {
                let Some(source) = source_of_rels(rels_name) else { continue };
                for rel in parse_relationships(rels_doc) {
                    let targets_picture = !rel.external
                        && resolve_target(&source, &rel.target).is_some_and(|t| t.eq_ignore_ascii_case(picture));
                    if !targets_picture {
                        continue;
                    }
                    let source_bytes = xml_parts
                        .get(&source)
                        .map(Vec::as_slice)
                        .or_else(|| self.package.part(&source).map(|p| p.data.as_slice()))
                        .unwrap_or_default();
                    if contains_bytes(source_bytes, format!("\"{}\"", rel.id).as_bytes()) {
                        used = true;
                    } else {
                        references.push((rels_name.clone(), rel.id));
                    }
                }
            }
            if !used {
                unused.push(picture);
                for (rels_name, id) in references {
                    let mut doc = self.edited[&rels_name].clone();
                    doc.root.children.retain(|n| {
                        !matches!(n, Node::Element(e) if e.is(PACKAGE_RELS_NS, "Relationship") && e.attr(None, "Id").as_deref() == Some(id.as_str()))
                    });
                    xml_parts.insert(rels_name, doc.to_bytes());
                }
            }
        }

        let mut replacements: Vec<(&str, &[u8])> = xml_parts.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        for (name, data) in &self.added {
            if !unused.contains(&name.as_str()) {
                replacements.push((name.as_str(), data.as_slice()));
            }
        }
        let bytes = self.package.write_with(&replacements)?;
        verify_saved(&bytes, replacements.iter().map(|(name, _)| *name))?;
        Ok(bytes)
    }

    /// Whether the document has unsaved edits since it was opened.
    pub fn is_modified(&self) -> bool {
        self.modified || !self.edited.is_empty() || !self.added.is_empty()
    }

    /// The underlying package (all parts as read).
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// Name of the main document part (usually `word/document.xml`).
    pub fn main_part(&self) -> &str {
        &self.main_part
    }

    /// The bytes of an image part that the main document, a header or a
    /// footer references as an image (`word/media/…`), for display. Other
    /// parts are not returned.
    pub fn image_part(&self, name: &str) -> Option<&[u8]> {
        let mut sources = vec![self.main_part.clone()];
        sources.extend(self.header_footer_parts());
        let referenced = sources.iter().any(|source| {
            self.part_rels(source).iter().any(|r| {
                r.is_type(rel_type::IMAGE)
                    && !r.external
                    && resolve_target(source, &r.target).is_some_and(|target| target.eq_ignore_ascii_case(name))
            })
        });
        if !referenced {
            return None;
        }
        if let Some((_, data)) = self.added.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Some(data.as_slice());
        }
        self.package.part(name).map(|p| p.data.as_slice())
    }

    /// Number of top-level blocks (paragraphs, tables and other content).
    pub fn block_count(&self) -> usize {
        block_positions(self.body(), &self.w.ns).len()
    }

    /// Keeps an uploaded picture so that a later splice can insert it (see
    /// [`Piece::Image`]). The file is recognized by its content (PNG, JPEG,
    /// GIF or BMP); it does not change the document by itself.
    pub fn stage_image(&mut self, data: Vec<u8>) -> Result<StagedImageInfo> {
        let (format, width_px, height_px) = media::sniff(&data)?;
        let staged_bytes: usize = self.staged.values().map(|s| s.data.len()).sum();
        if staged_bytes + data.len() > MAX_STAGED_BYTES {
            return Err(DocxError::TooLarge("too many pictures are waiting to be inserted".into()));
        }
        let id = format!("img{}", self.next_image);
        self.next_image += 1;
        let staged = StagedImage { id: id.clone(), format, width_px, height_px, data: Arc::new(data) };
        let info = staged.info();
        self.staged.insert(id, staged);
        Ok(info)
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
        let index = insertion_index(body, &w.ns, &positions, at);
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
        let saved = self.checkpoint();
        for (i, edit) in edits.iter().enumerate() {
            if let Err(e) = self.apply_one(edit) {
                self.rollback(saved);
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

    fn apply_one(&mut self, edit: &Edit) -> Result<()> {
        match edit {
            Edit::Splice(s) => self.splice_edit(s),
            Edit::FormatParagraphs { container, first, last, format } => {
                self.format_paragraphs_in(container, *first, *last, format)
            }
            Edit::InsertParagraph { at, text, style_id } => self.insert_paragraph(*at, text, style_id.as_deref()),
            Edit::DeleteBlock { block } => self.delete_block(*block),
            Edit::SetParagraphStyle { block, style_id } => self.set_paragraph_style(*block, style_id.as_deref()),
            Edit::SetParagraphAlignment { block, alignment } => self.set_paragraph_alignment(*block, *alignment),
            Edit::InsertTable { container, at, rows, columns } => self.insert_table(container, *at, *rows, *columns),
            Edit::InsertRow { container, table, row, after } => {
                self.with_table(container, *table, |tbl, w| table::insert_row(tbl, w, *row, *after))
            }
            Edit::DeleteRow { container, table, row } => {
                self.with_table(container, *table, |tbl, w| table::delete_row(tbl, w, *row))
            }
            Edit::InsertColumn { container, table, row, cell, after } => {
                self.with_table(container, *table, |tbl, w| table::insert_column(tbl, w, *row, *cell, *after))
            }
            Edit::DeleteColumn { container, table, row, cell } => {
                self.with_table(container, *table, |tbl, w| table::delete_column(tbl, w, *row, *cell))
            }
            Edit::SetTableBorders { container, table, borders } => self.with_table(container, *table, |tbl, w| {
                table::set_table_borders(tbl, w, *borders);
                Ok(())
            }),
            Edit::FormatCell { container, table, row, cell, format } => {
                format.validate()?;
                self.with_table(container, *table, |tbl, w| {
                    table::format_cell(table::table_cell_mut(tbl, w, *row, *cell)?, w, format);
                    Ok(())
                })
            }
            Edit::ResizeImage { container, block, inline, width_emu, height_emu } => {
                self.resize_image(container, *block, *inline, *width_emu, *height_emu)
            }
            Edit::FormatSection { format } => self.format_sections(format),
            Edit::AddHeaderFooter { kind } => self.ensure_header_footer(*kind).map(|_| ()),
            Edit::InsertPageNumbers { kind, alignment, with_total } => {
                self.insert_page_numbers(*kind, *alignment, *with_total)
            }
        }
    }

    fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            xml: self.xml.clone(),
            modified: self.modified,
            edited: self.edited.clone(),
            added: self.added.clone(),
            rels: self.rels.clone(),
            styles: self.styles.clone(),
        }
    }

    fn rollback(&mut self, saved: Checkpoint) {
        self.xml = saved.xml;
        self.modified = saved.modified;
        self.edited = saved.edited;
        self.added = saved.added;
        self.rels = saved.rels;
        self.styles = saved.styles;
    }

    /// Runs one edit; on error the document is left unchanged.
    fn transact(&mut self, f: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let saved = self.checkpoint();
        let result = f(self);
        if result.is_err() {
            self.rollback(saved);
        }
        result
    }

    /// Replaces paragraphs with new content; see [`Splice`]. On error the
    /// document is unchanged.
    pub fn splice(&mut self, splice: &Splice) -> Result<()> {
        self.transact(|doc| doc.splice_edit(splice))
    }

    fn splice_edit(&mut self, s: &Splice) -> Result<()> {
        // Pictures are added to the package first (rolled back if the
        // splice fails), so that the splice itself only rearranges XML.
        let mut prepared: Vec<Option<Element>> = vec![None; s.content.len()];
        if s.content.iter().any(|p| matches!(p, Piece::Image { .. })) {
            let source = self.container_source(&s.container)?;
            let w = self.container_names(&s.container)?;
            let mut next_id = self.next_drawing_id();
            for (i, piece) in s.content.iter().enumerate() {
                if let Piece::Image { image, width_emu, height_emu, description } = piece {
                    prepared[i] = Some(self.picture_run(
                        &source,
                        &w,
                        next_id,
                        image,
                        (*width_emu, *height_emu),
                        description.as_deref(),
                    )?);
                    next_id += 1;
                }
            }
        }
        let (container, w) = self.container_mut(&s.container)?;
        splice_in(container, &w, s.first, s.last, &s.content, &prepared)
    }

    /// Applies a paragraph format (style, alignment, indentation, spacing,
    /// list) to body paragraphs `first..=last`. Styles the document doesn't
    /// define are added from the built-in set (Title, Heading 1–3, …); list
    /// definitions are reused or added to the numbering part, which is
    /// created if the document has none. On error the document is
    /// unchanged.
    pub fn format_paragraphs(&mut self, first: usize, last: usize, format: &ParagraphFormat) -> Result<()> {
        self.transact(|doc| doc.format_paragraphs_in(&Container::body(), first, last, format))
    }

    fn format_paragraphs_in(
        &mut self,
        container: &Container,
        first: usize,
        last: usize,
        format: &ParagraphFormat,
    ) -> Result<()> {
        format.validate()?;
        {
            let (root, w) = self.container_mut(container)?;
            let positions = block_positions(root, &w.ns);
            if first > last || last >= positions.len() {
                return Err(DocxError::InvalidEdit(format!("paragraphs {first}..{last} are not in the document")));
            }
            for (block, &position) in positions.iter().enumerate().take(last + 1).skip(first) {
                match &root.children[position] {
                    Node::Element(el) if el.is(&w.ns, "p") => {}
                    _ => return Err(DocxError::InvalidEdit(format!("block {block} is not a paragraph"))),
                }
            }
        }
        if let Some(Some(style)) = &format.style_id {
            validate_style_id(style)?;
            self.ensure_style(style)?;
        }
        let num_id = match &format.list {
            Some(Some(list)) => Some(self.list_num_id(list.kind)?),
            _ => None,
        };
        let (root, w) = self.container_mut(container)?;
        let positions = block_positions(root, &w.ns);
        for &position in &positions[first..=last] {
            if let Node::Element(p) = &mut root.children[position] {
                apply_paragraph_format(p, &w, format, num_id.as_deref());
            }
        }
        Ok(())
    }

    // ----- Tables -----

    /// Inserts an empty table as block `at`. Empty paragraphs are added
    /// around it where another table would otherwise touch it (Word joins
    /// adjacent tables) or where it would end the container.
    fn insert_table(&mut self, container: &Container, at: usize, rows: usize, columns: usize) -> Result<()> {
        if container.cell.is_some() {
            return Err(DocxError::InvalidEdit("tables can't be inserted inside a table cell".into()));
        }
        let width = self.text_width();
        let style = self.styles.iter().any(|s| s.id == "TableGrid" && s.kind == "table").then_some("TableGrid");
        let (root, w) = self.container_mut(container)?;
        let tbl = table::build_table(&w, rows, columns, width, style)?;
        let positions = block_positions(root, &w.ns);
        if at > positions.len() {
            return Err(DocxError::InvalidEdit(format!("position {at} is past the end of the document")));
        }
        let is = |position: Option<&usize>, local: &str| {
            position.is_some_and(|&i| matches!(&root.children[i], Node::Element(e) if e.is(&w.ns, local)))
        };
        let table_before = at > 0 && is(positions.get(at - 1), "tbl");
        let paragraph_after = is(positions.get(at), "p");
        let index = insertion_index(root, &w.ns, &positions, at);
        let mut nodes = Vec::new();
        if table_before {
            nodes.push(Node::Element(w.element("p")));
        }
        nodes.push(Node::Element(tbl));
        if !paragraph_after {
            nodes.push(Node::Element(w.element("p")));
        }
        root.children.splice(index..index, nodes);
        Ok(())
    }

    fn with_table(
        &mut self,
        container: &Container,
        table: usize,
        f: impl FnOnce(&mut Element, &WNames) -> Result<()>,
    ) -> Result<()> {
        if container.cell.is_some() {
            return Err(DocxError::InvalidEdit("tables inside table cells can't be changed yet".into()));
        }
        let (root, w) = self.container_mut(container)?;
        f(table_in(root, &w.ns, table)?, &w)
    }

    /// The width available for text on the last section's pages, in twips.
    fn text_width(&self) -> u32 {
        let num = |el: Option<&Element>, name: &str| {
            el.and_then(|e| e.attr(Some(&self.w.ns), name)).and_then(|v| v.parse::<i64>().ok())
        };
        let sect = self.body().child(&self.w.ns, "sectPr");
        let width = num(sect.and_then(|s| s.child(&self.w.ns, "pgSz")), "w").unwrap_or(11906);
        let margins = sect.and_then(|s| s.child(&self.w.ns, "pgMar"));
        let used = num(margins, "left").unwrap_or(1440) + num(margins, "right").unwrap_or(1440);
        (width - used).clamp(1440, 31680) as u32
    }

    // ----- Pictures -----

    /// Adds a staged picture to the package (once, however often it is
    /// inserted) with a relationship from `source`, and returns the run
    /// that shows it.
    fn picture_run(
        &mut self,
        source: &str,
        w: &WNames,
        drawing_id: u32,
        image: &str,
        (width_emu, height_emu): (i64, i64),
        description: Option<&str>,
    ) -> Result<Element> {
        media::validate_size(width_emu, height_emu)?;
        self.refuse_strict("Pictures")?;
        let description = description.unwrap_or("");
        validate_text(description)?;
        if description.chars().count() > 1000 {
            return Err(DocxError::InvalidEdit("picture descriptions can be at most 1000 characters".into()));
        }
        let staged = self.staged.get(image).cloned().ok_or_else(|| {
            DocxError::InvalidEdit("the picture to insert is no longer available; insert it again".into())
        })?;
        let part = match self.added.iter().find(|(_, data)| Arc::ptr_eq(data, &staged.data)) {
            Some((name, _)) => name.clone(),
            None => {
                let dir = format!("{}media", part_dir(&self.main_part));
                let name = self.free_part_name(&dir, "image", staged.format.extension());
                self.ensure_default_content_type(staged.format.extension(), staged.format.content_type())?;
                self.added.insert(name.clone(), staged.data.clone());
                name
            }
        };
        let existing = self.part_rels(source).into_iter().find(|r| {
            r.is_type(rel_type::IMAGE)
                && !r.external
                && resolve_target(source, &r.target).is_some_and(|t| t.eq_ignore_ascii_case(&part))
        });
        let rel_id = match existing {
            Some(rel) => rel.id,
            None => self.add_relationship(source, IMAGE_REL_TYPE, &relative_target(source, &part))?,
        };
        let file_name = part.rsplit('/').next().unwrap_or(&part).to_string();
        Ok(media::picture_run(
            w,
            &media::Picture {
                rel_id: &rel_id,
                id: drawing_id,
                file_name: &file_name,
                description,
                width_emu,
                height_emu,
            },
        ))
    }

    /// A drawing object id not used in the document, its headers or footers.
    fn next_drawing_id(&self) -> u32 {
        let mut max = media::max_drawing_id(&self.xml.root);
        for part in self.header_footer_parts() {
            if let Some(doc) = self.part_xml(&part) {
                max = max.max(media::max_drawing_id(&doc.root));
            }
        }
        max + 1
    }

    fn resize_image(
        &mut self,
        container: &Container,
        block: usize,
        inline: usize,
        width_emu: i64,
        height_emu: i64,
    ) -> Result<()> {
        media::validate_size(width_emu, height_emu)?;
        let (root, w) = self.container_mut(container)?;
        let p = paragraph_in(root, &w.ns, block)?;
        let run = p
            .elements_mut()
            .filter(|e| is_inline(e, &w.ns))
            .nth(inline)
            .filter(|e| e.is(&w.ns, "r"))
            .ok_or_else(|| DocxError::InvalidEdit(format!("inline {inline} of block {block} is not a picture")))?;
        let picture = media::run_picture_mut(run, &w.ns)
            .ok_or_else(|| DocxError::InvalidEdit(format!("inline {inline} of block {block} is not a picture")))?;
        media::set_picture_size(picture, width_emu, height_emu);
        Ok(())
    }

    // ----- Page layout, headers and footers -----

    /// Applies a page layout change to every section.
    fn format_sections(&mut self, format: &SectionFormat) -> Result<()> {
        format.validate()?;
        self.ensure_body_section();
        let w = self.w.clone();
        let mut result = Ok(());
        for_each_section(self.body_mut(), &w.ns, &mut |sect| {
            if result.is_ok() {
                result = apply_section_format(sect, &w, format);
            }
        });
        result
    }

    /// Gives the body its final section properties if it has none.
    /// Marks the main part edited (all section changes go through here).
    fn ensure_body_section(&mut self) {
        self.modified = true;
        let w = self.w.clone();
        let body = self.body_mut();
        if body.child(&w.ns, "sectPr").is_none() {
            body.children.push(Node::Element(w.element("sectPr")));
        }
    }

    /// The relationship id of the header or footer shown on the last
    /// section's pages: its own default reference, or the one it inherits.
    fn effective_reference(&self, kind: HeaderFooterKind) -> Option<String> {
        let local = reference_name(kind);
        let mut current = None;
        for sect in sections(self.body(), &self.w.ns) {
            let found = sect
                .elements()
                .find(|e| e.is(&self.w.ns, local) && e.attr(Some(&self.w.ns), "type").is_none_or(|t| t == "default"));
            if let Some(reference) = found {
                current = reference.attr(Some(R_NS), "id").or_else(|| reference.attr(Some(R_NS_STRICT), "id"));
            }
        }
        current
    }

    fn effective_part(&self, kind: HeaderFooterKind) -> Option<String> {
        let id = self.effective_reference(kind)?;
        let rel = self.rels.iter().find(|r| r.id == id && !r.external)?;
        resolve_target(&self.main_part, &rel.target)
    }

    fn header_footer_content(&self, kind: HeaderFooterKind) -> Option<HeaderFooter> {
        let part = self.effective_part(kind)?;
        let doc = self.part_xml(&part)?;
        let rels = self.part_rels(&part);
        let ctx = ReadContext { w: &self.w.ns, main_part: &part, rels: &rels };
        let blocks = read_blocks(&doc.root, &ctx, &mut Stats::default());
        Some(HeaderFooter { part, blocks })
    }

    /// The header or footer shown on every page, creating an empty one
    /// (part, relationship, content type, and a reference in each section
    /// that has none) if the document has none. Returns its part name.
    fn ensure_header_footer(&mut self, kind: HeaderFooterKind) -> Result<String> {
        if let Some(part) = self.effective_part(kind) {
            return Ok(part);
        }
        self.refuse_strict("Headers and footers")?;
        let (stem, root_name, style, content_type, rel_type) = match kind {
            HeaderFooterKind::Header => ("header", "hdr", "Header", HEADER_CONTENT_TYPE, HEADER_REL_TYPE),
            HeaderFooterKind::Footer => ("footer", "ftr", "Footer", FOOTER_CONTENT_TYPE, FOOTER_REL_TYPE),
        };
        self.ensure_style(style)?;
        let paragraph = if self.styles.iter().any(|s| s.id == style) {
            format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr></w:p>"#)
        } else {
            "<w:p/>".to_string()
        };
        let name = self.free_part_name(part_dir(&self.main_part).trim_end_matches('/'), stem, "xml");
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:{root_name} xmlns:w="{W_NS}" xmlns:r="{R_NS}">{paragraph}</w:{root_name}>"#
        );
        let doc = XmlDocument::parse(xml.as_bytes()).map_err(|e| DocxError::Write(e.0))?;
        self.edited.insert(name.clone(), doc);
        self.add_content_type_override(&name, content_type)?;
        let main = self.main_part.clone();
        let id = self.add_relationship(&main, rel_type, &relative_target(&main, &name))?;

        // Every section without its own header (footer) gets the new one.
        let r_prefix = self.ensure_namespace_declared(R_NS, "r");
        let w = self.w.clone();
        self.ensure_body_section();
        let local = reference_name(kind);
        for_each_section(self.body_mut(), &w.ns, &mut |sect| {
            let has_default = sect
                .elements()
                .any(|e| e.is(&w.ns, local) && e.attr(Some(&w.ns), "type").is_none_or(|t| t == "default"));
            if !has_default {
                let mut reference = w.element(local);
                w.set_attr(&mut reference, "type", "default");
                reference.set_attr(&format!("{r_prefix}:id"), Some(R_NS), &id);
                insert_reference(sect, &w, reference);
            }
        });
        Ok(name)
    }

    /// Adds a paragraph with a page number field (and the page count) to the
    /// header or footer.
    fn insert_page_numbers(&mut self, kind: HeaderFooterKind, alignment: Alignment, with_total: bool) -> Result<()> {
        let part = self.ensure_header_footer(kind)?;
        let style = match kind {
            HeaderFooterKind::Header => "Header",
            HeaderFooterKind::Footer => "Footer",
        };
        self.ensure_style(style)?;
        let has_style = self.styles.iter().any(|s| s.id == style);
        let (container, w) = self.container_mut(&Container::part(&part))?;
        if has_page_field(container, &w.ns) {
            let what = if kind == HeaderFooterKind::Header { "header" } else { "footer" };
            return Err(DocxError::InvalidEdit(format!("the {what} already has page numbers")));
        }
        let field = |code: &str| {
            let mut fld = w.element("fldSimple");
            w.set_attr(&mut fld, "instr", &format!(" {code} "));
            fld.children.push(Node::Element(build_run(&w, None, "1")));
            Node::Element(fld)
        };
        let mut content = Vec::new();
        if with_total {
            content.push(Node::Element(build_run(&w, None, "Page ")));
            content.push(field("PAGE"));
            content.push(Node::Element(build_run(&w, None, " of ")));
            content.push(field("NUMPAGES"));
        } else {
            content.push(field("PAGE"));
        }
        let format = ParagraphFormat {
            style_id: has_style.then(|| Some(style.to_string())),
            alignment: Some(Some(alignment)),
            ..ParagraphFormat::default()
        };

        // A header or footer that is just an empty paragraph gets the page
        // number in it; otherwise a paragraph is added at the end.
        let positions = block_positions(container, &w.ns);
        let empty = positions.len() == 1
            && matches!(&container.children[positions[0]], Node::Element(p) if p.is(&w.ns, "p") && !p.elements().any(|e| is_inline(e, &w.ns)));
        if empty {
            if let Node::Element(p) = &mut container.children[positions[0]] {
                apply_paragraph_format(p, &w, &format, None);
                p.children.extend(content);
            }
        } else {
            let mut p = w.element("p");
            apply_paragraph_format(&mut p, &w, &format, None);
            p.children.extend(content);
            container.children.push(Node::Element(p));
        }
        Ok(())
    }

    // ----- Styles and lists -----

    /// Makes sure style `id` exists, adding a built-in definition if needed.
    fn ensure_style(&mut self, id: &str) -> Result<()> {
        if self.styles.iter().any(|s| s.id == id) {
            return Ok(());
        }
        let builtin =
            builtin_style(id).ok_or_else(|| DocxError::InvalidEdit(format!("the document has no style named {id}")))?;
        let Some(name) = self.related_part(rel_type::STYLES) else {
            // No style sheet: Word shows the paragraph in the default style.
            return Ok(());
        };
        let ns = self.w.ns.clone();
        let doc = self.edited_xml(&name)?;
        if doc.prefix_for(&ns).as_deref() != Some("w") {
            return Ok(()); // unusual prefix: keep the style sheet untouched
        }
        let style_model = crate::model::Style {
            id: id.to_string(),
            name: builtin_name(&builtin, &ns),
            kind: builtin.attr(Some(&ns), "type").unwrap_or_else(|| "paragraph".into()),
            based_on: builtin.child(&ns, "basedOn").and_then(|b| b.attr(Some(&ns), "val")),
            is_default: false,
        };
        doc.root.children.push(Node::Element(builtin));
        self.styles.push(style_model);
        Ok(())
    }

    /// The numId of a bullet or numbered list, creating the definition (and
    /// the numbering part) if necessary.
    fn list_num_id(&mut self, kind: ListKind) -> Result<String> {
        let name = match self.related_part(rel_type::NUMBERING) {
            Some(name) => name,
            None => self.create_numbering_part()?,
        };
        let ns = self.w.ns.clone();
        let doc = self.edited_xml(&name)?;
        let prefix = doc.prefix_for(&ns).unwrap_or_default();
        Ok(numbering::find_or_create(doc, &ns, &prefix, kind))
    }

    /// Adds `word/numbering.xml` with its relationship and content type.
    fn create_numbering_part(&mut self) -> Result<String> {
        let dir = part_dir(&self.main_part).trim_end_matches('/').to_string();
        let join = |file: &str| if dir.is_empty() { file.to_string() } else { format!("{dir}/{file}") };
        let mut name = join("numbering.xml");
        let mut n = 1;
        while self.part_exists(&name) {
            n += 1;
            name = join(&format!("numbering{n}.xml"));
        }
        let doc = XmlDocument::parse(&numbering::empty_part(&self.w.ns))
            .map_err(|e| DocxError::Write(format!("numbering template: {}", e.0)))?;
        self.edited.insert(name.clone(), doc);
        let main = self.main_part.clone();
        self.add_relationship(&main, NUMBERING_REL_TYPE, &relative_target(&main, &name))?;
        self.add_content_type_override(&name, NUMBERING_CONTENT_TYPE)?;
        Ok(name)
    }

    // ----- Parts, relationships and content types -----

    /// A secondary part's XML for editing (parsed from the package once).
    fn edited_xml(&mut self, name: &str) -> Result<&mut XmlDocument> {
        if !self.edited.contains_key(name) {
            let part = self
                .package
                .part(name)
                .ok_or_else(|| DocxError::InvalidEdit(format!("the document has no part {name}")))?;
            let doc = parse_part(name, &part.data)?;
            self.edited.insert(name.to_string(), doc);
        }
        Ok(self.edited.get_mut(name).expect("inserted above"))
    }

    /// A part's current XML: edited, or parsed from the package.
    fn part_xml(&self, name: &str) -> Option<Cow<'_, XmlDocument>> {
        if let Some(doc) = self.edited.get(name) {
            return Some(Cow::Borrowed(doc));
        }
        self.package.part(name).and_then(|p| XmlDocument::parse(&p.data).ok()).map(Cow::Owned)
    }

    /// The relationships of a part (as currently edited).
    fn part_rels(&self, source: &str) -> Vec<Relationship> {
        if source == self.main_part {
            return self.rels.clone();
        }
        self.part_xml(&rels_part_for(source)).map(|doc| parse_relationships(&doc)).unwrap_or_default()
    }

    fn part_exists(&self, name: &str) -> bool {
        self.package.part(name).is_some()
            || self.edited.keys().any(|n| n.eq_ignore_ascii_case(name))
            || self.added.keys().any(|n| n.eq_ignore_ascii_case(name))
    }

    /// `dir/stemN.ext` for the first N not used by another part.
    fn free_part_name(&self, dir: &str, stem: &str, ext: &str) -> String {
        let join = |file: String| if dir.is_empty() { file } else { format!("{dir}/{file}") };
        (1..).map(|n| join(format!("{stem}{n}.{ext}"))).find(|name| !self.part_exists(name)).expect("unbounded")
    }

    /// Adds a relationship from `source` and returns its id.
    fn add_relationship(&mut self, source: &str, rel_type: &str, target: &str) -> Result<String> {
        let rels_name = rels_part_for(source);
        if self.package.part(&rels_name).is_none() && !self.edited.contains_key(&rels_name) {
            let empty = format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="{PACKAGE_RELS_NS}"></Relationships>"#
            );
            let doc = XmlDocument::parse(empty.as_bytes()).map_err(|e| DocxError::Write(e.0))?;
            self.edited.insert(rels_name.clone(), doc);
        }
        let rels = self.edited_xml(&rels_name)?;
        let existing = parse_relationships(rels);
        let id = (existing.len() + 1..)
            .map(|n| format!("rId{n}"))
            .find(|candidate| !existing.iter().any(|r| r.id == *candidate))
            .expect("unbounded");
        let prefix = rels.prefix_for(PACKAGE_RELS_NS).unwrap_or_default();
        let mut rel = Element::new(&prefix, "Relationship", Some(PACKAGE_RELS_NS));
        rel.set_attr("Id", None, &id);
        rel.set_attr("Type", None, rel_type);
        rel.set_attr("Target", None, target);
        rels.root.children.push(Node::Element(rel));
        if source == self.main_part {
            self.rels.push(Relationship {
                id: id.clone(),
                rel_type: rel_type.into(),
                target: target.into(),
                external: false,
            });
        }
        Ok(id)
    }

    fn add_content_type_override(&mut self, part: &str, content_type: &str) -> Result<()> {
        let types = self.edited_xml("[Content_Types].xml")?;
        let prefix = types.prefix_for(CONTENT_TYPES_NS).unwrap_or_default();
        let mut over = Element::new(&prefix, "Override", Some(CONTENT_TYPES_NS));
        over.set_attr("PartName", None, &format!("/{part}"));
        over.set_attr("ContentType", None, content_type);
        types.root.children.push(Node::Element(over));
        Ok(())
    }

    /// Declares the content type of a file extension unless it already is.
    fn ensure_default_content_type(&mut self, ext: &str, content_type: &str) -> Result<()> {
        let types = self.edited_xml("[Content_Types].xml")?;
        let declared = types.root.elements().any(|e| {
            e.is(CONTENT_TYPES_NS, "Default") && e.attr(None, "Extension").is_some_and(|x| x.eq_ignore_ascii_case(ext))
        });
        if declared {
            return Ok(());
        }
        let prefix = types.prefix_for(CONTENT_TYPES_NS).unwrap_or_default();
        let mut default = Element::new(&prefix, "Default", Some(CONTENT_TYPES_NS));
        default.set_attr("Extension", None, ext);
        default.set_attr("ContentType", None, content_type);
        // Defaults conventionally come before overrides.
        let at = types
            .root
            .children
            .iter()
            .position(|n| matches!(n, Node::Element(e) if e.is(CONTENT_TYPES_NS, "Override")))
            .unwrap_or(types.root.children.len());
        types.root.children.insert(at, Node::Element(default));
        Ok(())
    }

    /// Header and footer parts the main document relates to.
    fn header_footer_parts(&self) -> Vec<String> {
        self.rels
            .iter()
            .filter(|r| !r.external && (r.is_type(rel_type::HEADER) || r.is_type(rel_type::FOOTER)))
            .filter_map(|r| resolve_target(&self.main_part, &r.target))
            .collect()
    }

    /// Prefix for a namespace on the main document's root, declaring
    /// `preferred` for it if the root doesn't declare it yet.
    fn ensure_namespace_declared(&mut self, ns: &str, preferred: &str) -> String {
        match self.xml.prefix_for(ns) {
            Some(prefix) if !prefix.is_empty() => prefix,
            _ => {
                self.xml.root.set_attr(&format!("xmlns:{preferred}"), None, ns);
                self.modified = true;
                preferred.to_string()
            }
        }
    }

    fn refuse_strict(&self, what: &str) -> Result<()> {
        if self.w.ns == W_NS_STRICT {
            return Err(DocxError::InvalidEdit(format!(
                "{what} can't be added to Strict Open XML documents; save the file as a regular Word document first"
            )));
        }
        Ok(())
    }

    // ----- Containers -----

    /// The part whose relationships a container's content uses.
    fn container_source(&self, container: &Container) -> Result<String> {
        match &container.part {
            None => Ok(self.main_part.clone()),
            Some(part) => self.header_footer_part(part),
        }
    }

    /// A header or footer part name as stored in the package.
    fn header_footer_part(&self, name: &str) -> Result<String> {
        let name = name.trim_start_matches('/');
        self.header_footer_parts()
            .into_iter()
            .find(|p| p.eq_ignore_ascii_case(name))
            .ok_or_else(|| DocxError::InvalidEdit(format!("the document has no header or footer {name}")))
    }

    fn container_names(&mut self, container: &Container) -> Result<WNames> {
        self.container_mut(container).map(|(_, w)| w)
    }

    /// The element whose children are the container's blocks, and the names
    /// to write WordprocessingML elements with in it.
    fn container_mut(&mut self, container: &Container) -> Result<(&mut Element, WNames)> {
        let (root, w) = match &container.part {
            None => {
                self.modified = true;
                let w = self.w.clone();
                (self.body_mut(), w)
            }
            Some(part) => {
                let name = self.header_footer_part(part)?;
                let ns = self.w.ns.clone();
                let doc = self.edited_xml(&name)?;
                let is_header_footer = doc.root.namespace.as_deref() == Some(ns.as_str())
                    && ["hdr", "ftr"].contains(&doc.root.local_name());
                if !is_header_footer {
                    return Err(DocxError::InvalidEdit(format!(
                        "{name} is not a header or footer the editor can change"
                    )));
                }
                let w = names_in(doc, &ns);
                (&mut doc.root, w)
            }
        };
        match container.cell {
            None => Ok((root, w)),
            Some(cell) => {
                let tc = cell_in(root, &w, cell)?;
                Ok((tc, w))
            }
        }
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
        paragraph_in(self.body_mut(), &ns, block)
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

/// Replaces paragraphs `first..=last` of `container` with `content`; see
/// [`Splice`]. `prepared[i]` is the run built for an image piece
/// `content[i]`.
fn splice_in(
    container: &mut Element,
    w: &WNames,
    first: usize,
    last: usize,
    content: &[Piece],
    prepared: &[Option<Element>],
) -> Result<()> {
    let positions = block_positions(container, &w.ns);
    if first > last || last >= positions.len() {
        return Err(DocxError::InvalidEdit(format!(
            "paragraphs {first}..{last} are not in the document ({} blocks)",
            positions.len()
        )));
    }

    // The paragraphs involved, and the virtual sequence of their content.
    let mut paragraphs: Vec<&Element> = Vec::with_capacity(last - first + 1);
    for (block, &position) in positions.iter().enumerate().take(last + 1).skip(first) {
        match &container.children[position] {
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
                    return Err(DocxError::InvalidEdit("kept content must be listed once, in document order".into()));
                }
                kept.push(r);
            }
            Piece::Text { text, like, format } => {
                validate_text(text)?;
                if let Some(f) = format {
                    f.validate()?;
                }
                if let Some(r) = like
                    && !inline_at(*r)?.is(&w.ns, "r")
                {
                    return Err(DocxError::InvalidEdit(format!(
                        "inline {} of block {} is not a run, so its formatting cannot be copied",
                        r.inline, r.block
                    )));
                }
            }
            Piece::Break | Piece::PageBreak | Piece::Image { .. } => {}
        }
    }

    // Build the new paragraphs' content by walking the sequence: kept
    // inlines and markers stay in order; new content is flushed at the
    // first removed inline (where the edit happened) or before the next
    // kept inline.
    let mut outputs: Vec<Vec<Node>> = vec![Vec::new()];
    let mut cursor = 0;
    let flush_until = |end: usize, cursor: &mut usize, outputs: &mut Vec<Vec<Node>>| -> Result<()> {
        while *cursor < end {
            let current = outputs.last_mut().expect("never empty");
            match &content[*cursor] {
                Piece::Text { text, like, format } if !text.is_empty() => {
                    let mut rpr = match like {
                        Some(r) => inline_at(*r)?.child(&w.ns, "rPr").cloned(),
                        None => None,
                    };
                    if let Some(f) = format {
                        rpr = apply_run_format(rpr, w, f);
                    }
                    current.push(Node::Element(build_run(w, rpr, text)));
                }
                Piece::PageBreak => {
                    let mut run = w.element("r");
                    let mut br = w.element("br");
                    w.set_attr(&mut br, "type", "page");
                    run.children.push(Node::Element(br));
                    current.push(Node::Element(run));
                }
                Piece::Image { .. } => {
                    let run = prepared.get(*cursor).cloned().flatten().expect("pictures are prepared");
                    current.push(Node::Element(run));
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
        remove_child(ppr, w, "sectPr");
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
            *ensure_child(ppr, w, PPR_ORDER, "sectPr") = sect.clone();
        }
        if let Some(ppr) = ppr.filter(|ppr| !ppr.children.is_empty() || index == 0) {
            p.children.push(Node::Element(ppr));
        }
        p.children.extend(nodes);
        new_paragraphs.push(Node::Element(p));
    }

    for block in (first..=last).rev() {
        container.children.remove(positions[block]);
    }
    let at = positions[first];
    container.children.splice(at..at, new_paragraphs);
    Ok(())
}

/// Reads a just-written archive back: it must open as a document and the
/// rewritten XML parts must be well formed.
fn verify_saved<'a>(bytes: &[u8], rewritten: impl Iterator<Item = &'a str>) -> Result<()> {
    let fail =
        |what: String| DocxError::Write(format!("the saved file did not pass its check ({what}); nothing was written"));
    let reopened = Document::open(bytes).map_err(|e| fail(e.to_string()))?;
    for name in rewritten {
        let Some(part) = reopened.package.part(name) else {
            return Err(fail(format!("{name} is missing")));
        };
        if name.ends_with(".xml") || name.ends_with(".rels") {
            XmlDocument::parse(&part.data).map_err(|e| fail(format!("{name}: {e}")))?;
        }
    }
    Ok(())
}

/// Where to insert a new block so that it becomes block `at`: before the
/// current block `at`, or at the end (before the body's final section
/// properties).
fn insertion_index(container: &Element, w: &str, positions: &[usize], at: usize) -> usize {
    match positions.get(at) {
        Some(&i) => i,
        None => container
            .children
            .iter()
            .rposition(|n| matches!(n, Node::Element(e) if e.is(w, "sectPr")))
            .unwrap_or_else(|| positions.last().map_or(container.children.len(), |&i| i + 1)),
    }
}

fn paragraph_in<'a>(container: &'a mut Element, w: &str, block: usize) -> Result<&'a mut Element> {
    let positions = block_positions(container, w);
    let &index = positions.get(block).ok_or_else(|| DocxError::InvalidEdit(format!("there is no block {block}")))?;
    match &mut container.children[index] {
        Node::Element(el) if el.is(w, "p") => Ok(el),
        _ => Err(DocxError::InvalidEdit(format!("block {block} is not a paragraph"))),
    }
}

fn table_in<'a>(container: &'a mut Element, w: &str, block: usize) -> Result<&'a mut Element> {
    let positions = block_positions(container, w);
    let &index = positions.get(block).ok_or_else(|| DocxError::InvalidEdit(format!("there is no block {block}")))?;
    match &mut container.children[index] {
        Node::Element(el) if el.is(w, "tbl") => Ok(el),
        _ => Err(DocxError::InvalidEdit(format!("block {block} is not a table"))),
    }
}

fn cell_in<'a>(container: &'a mut Element, w: &WNames, cell: CellRef) -> Result<&'a mut Element> {
    let tbl = table_in(container, &w.ns, cell.table)?;
    table::table_cell_mut(tbl, w, cell.row, cell.cell)
}

/// Element names for a header or footer part, declaring a `w:` prefix for
/// attributes when the part uses WordprocessingML as its default namespace.
fn names_in(doc: &mut XmlDocument, ns: &str) -> WNames {
    let prefix = doc.root.name.split_once(':').map_or("", |(p, _)| p).to_string();
    let attr_prefix = if !prefix.is_empty() {
        prefix.clone()
    } else {
        match doc.prefix_for(ns).filter(|p| !p.is_empty()) {
            Some(p) => p,
            None => {
                doc.root.set_attr("xmlns:w", None, ns);
                "w".to_string()
            }
        }
    };
    WNames { ns: ns.to_string(), prefix, attr_prefix }
}

/// Section properties in document order: those ending sections inside the
/// body (on paragraphs, also within content controls), then the body's
/// final ones. Tables can't contain section breaks.
fn sections<'a>(body: &'a Element, w: &str) -> Vec<&'a Element> {
    fn walk<'a>(el: &'a Element, w: &str, out: &mut Vec<&'a Element>) {
        for child in el.elements() {
            if child.is(w, "p") {
                if let Some(sect) = section_properties(child, w) {
                    out.push(sect);
                }
            } else if child.is(w, "sectPr") {
                out.push(child);
            } else if !child.is(w, "tbl") {
                walk(child, w, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(body, w, &mut out);
    out
}

/// Calls `f` for every section's properties, in document order.
fn for_each_section(body: &mut Element, w: &str, f: &mut dyn FnMut(&mut Element)) {
    for child in body.elements_mut() {
        if child.is(w, "p") {
            if let Some(sect) = child.child_mut(w, "pPr").and_then(|ppr| ppr.child_mut(w, "sectPr")) {
                f(sect);
            }
        } else if child.is(w, "sectPr") {
            f(child);
        } else if !child.is(w, "tbl") {
            for_each_section(child, w, f);
        }
    }
}

fn reference_name(kind: HeaderFooterKind) -> &'static str {
    match kind {
        HeaderFooterKind::Header => "headerReference",
        HeaderFooterKind::Footer => "footerReference",
    }
}

/// Whether a header or footer already shows page numbers (a `PAGE` field).
fn has_page_field(el: &Element, w: &str) -> bool {
    let is_page = |code: &str| code.split_whitespace().next().is_some_and(|c| c.eq_ignore_ascii_case("PAGE"));
    el.elements().any(|child| {
        (child.is(w, "fldSimple") && child.attr(Some(w), "instr").is_some_and(|i| is_page(&i)))
            || (child.is(w, "instrText") && is_page(&child.text()))
            || has_page_field(child, w)
    })
}

/// The folder of a part with a trailing slash (`word/document.xml` →
/// `word/`), or `""` at the package root.
fn part_dir(part: &str) -> &str {
    part.rfind('/').map_or("", |i| &part[..=i])
}

/// A relationship target from `source` to `part`: relative within the same
/// folder, otherwise absolute.
fn relative_target(source: &str, part: &str) -> String {
    match part.strip_prefix(part_dir(source)) {
        Some(rest) => rest.to_string(),
        None => format!("/{part}"),
    }
}

/// `word/_rels/document.xml.rels` → `word/document.xml`.
fn source_of_rels(rels: &str) -> Option<String> {
    let (dir, file) = rels.rsplit_once("_rels/")?;
    let file = file.strip_suffix(".rels")?;
    Some(format!("{dir}{file}"))
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Styles the editor can add to documents that lack them, taken from the
/// blank document's style sheet.
fn builtin_style(id: &str) -> Option<Element> {
    let sheet = XmlDocument::parse(BLANK_STYLES.as_bytes()).ok()?;
    let w = W_NS;
    sheet.root.elements().find(|e| e.is(w, "style") && e.attr(Some(w), "styleId").as_deref() == Some(id)).cloned()
}

fn builtin_name(style: &Element, w: &str) -> String {
    style.child(w, "name").and_then(|n| n.attr(Some(w), "val")).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_targets_and_rels_sources() {
        assert_eq!(relative_target("word/document.xml", "word/media/image1.png"), "media/image1.png");
        assert_eq!(relative_target("word/header1.xml", "word/media/image1.png"), "media/image1.png");
        assert_eq!(relative_target("doc/main.xml", "word/media/image1.png"), "/word/media/image1.png");
        assert_eq!(relative_target("document.xml", "media/image1.png"), "media/image1.png");
        assert_eq!(source_of_rels("word/_rels/document.xml.rels").as_deref(), Some("word/document.xml"));
        assert_eq!(source_of_rels("_rels/.rels").as_deref(), Some(""));
        assert_eq!(part_dir("word/document.xml"), "word/");
        assert_eq!(part_dir("document.xml"), "");
    }

    #[test]
    fn saved_files_are_checked_before_they_are_returned() {
        let good = Document::new_blank().save().unwrap();
        assert!(verify_saved(&good, ["word/document.xml"].into_iter()).is_ok());

        let not_a_docx = verify_saved(b"PK broken", std::iter::empty()).unwrap_err();
        assert!(matches!(&not_a_docx, DocxError::Write(m) if m.contains("did not pass its check")), "{not_a_docx}");
        let missing = verify_saved(&good, ["word/footer9.xml"].into_iter()).unwrap_err();
        assert!(missing.to_string().contains("word/footer9.xml is missing"), "{missing}");

        let mut package = Package::read(&good, Limits::default()).unwrap();
        package.set_part("word/styles.xml", b"<w:styles><unclosed></w:styles>".to_vec());
        let broken = package.write().unwrap();
        let err = verify_saved(&broken, ["word/styles.xml"].into_iter()).unwrap_err();
        assert!(err.to_string().contains("word/styles.xml"), "{err}");
    }
}
