//! Builds the read model from the main document XML.

use crate::model::*;
use crate::opc::{Relationship, rel_type, resolve_target};
use crate::xml::{Element, XmlDocument};

/// WordprocessingML namespaces (transitional and strict).
pub const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const W_NS_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
/// Relationship-id attribute namespaces (transitional and strict).
pub const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const R_NS_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";

/// Body-level elements that are invisible markers rather than content. They
/// are preserved but are not blocks.
const BODY_MARKERS: [&str; 10] = [
    "bookmarkStart",
    "bookmarkEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "permStart",
    "permEnd",
    "proofErr",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeEnd",
];

/// Paragraph-level invisible markers.
const INLINE_MARKERS: [&str; 12] = [
    "bookmarkStart",
    "bookmarkEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "permStart",
    "permEnd",
    "proofErr",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
    "customXmlInsRangeStart",
];

/// Names and context needed to interpret the main document part.
pub struct ReadContext<'a> {
    pub w: &'a str,
    pub main_part: &'a str,
    pub rels: &'a [Relationship],
}

impl ReadContext<'_> {
    fn r_attr(&self, el: &Element, local: &str) -> Option<String> {
        el.attr(Some(R_NS), local).or_else(|| el.attr(Some(R_NS_STRICT), local))
    }

    fn w_val(&self, el: &Element) -> Option<String> {
        el.attr(Some(self.w), "val")
    }

    fn w_attr(&self, el: &Element, local: &str) -> Option<String> {
        el.attr(Some(self.w), local)
    }

    fn rel(&self, id: &str) -> Option<&Relationship> {
        self.rels.iter().find(|r| r.id == id)
    }
}

/// Indices into `body.children` of the elements that are blocks: every
/// element except the final `sectPr` and invisible markers.
pub fn block_positions(body: &Element, w: &str) -> Vec<usize> {
    body.children
        .iter()
        .enumerate()
        .filter_map(|(i, node)| match node {
            crate::xml::Node::Element(el) if is_block(el, w) => Some(i),
            _ => None,
        })
        .collect()
}

fn is_block(el: &Element, w: &str) -> bool {
    if el.namespace.as_deref() == Some(w) {
        let local = el.local_name();
        return local != "sectPr" && !BODY_MARKERS.contains(&local);
    }
    true
}

/// Builds the model (without styles and warnings, which the caller adds).
pub fn read_document(doc: &XmlDocument, ctx: &ReadContext<'_>) -> DocumentModel {
    let body = doc.root.child(ctx.w, "body");
    let mut stats = Stats::default();
    let blocks = body.map(|b| read_blocks(b, ctx, &mut stats)).unwrap_or_default();
    let section = body.and_then(|b| b.child(ctx.w, "sectPr")).map(|s| read_section(s, ctx));

    let mut headers = Vec::new();
    let mut footers = Vec::new();
    for rel in ctx.rels.iter().filter(|r| !r.external) {
        let list = if rel.is_type(rel_type::HEADER) {
            &mut headers
        } else if rel.is_type(rel_type::FOOTER) {
            &mut footers
        } else {
            continue;
        };
        if let Some(part) = resolve_target(ctx.main_part, &rel.target)
            && !list.contains(&part)
        {
            list.push(part);
        }
    }
    headers.sort();
    footers.sort();

    DocumentModel { blocks, section, styles: Vec::new(), headers, footers, stats, warnings: Vec::new() }
}

fn read_blocks(container: &Element, ctx: &ReadContext<'_>, stats: &mut Stats) -> Vec<Block> {
    let mut blocks = Vec::new();
    for el in container.elements() {
        if !is_block(el, ctx.w) || el.is(ctx.w, "tcPr") {
            continue;
        }
        let block = if el.is(ctx.w, "p") {
            stats.paragraphs += 1;
            Block::Paragraph(read_paragraph(el, ctx, stats))
        } else if el.is(ctx.w, "tbl") {
            stats.tables += 1;
            Block::Table(read_table(el, ctx, stats))
        } else {
            stats.preserved_elements += 1;
            let text = visible_text(el, ctx.w);
            stats.words += count_words(&text);
            Block::Other { kind: el.local_name().to_string(), text }
        };
        blocks.push(block);
    }
    blocks
}

fn read_paragraph(p: &Element, ctx: &ReadContext<'_>, stats: &mut Stats) -> Paragraph {
    let mut paragraph = Paragraph::default();
    if let Some(ppr) = p.child(ctx.w, "pPr") {
        read_paragraph_props(ppr, ctx, &mut paragraph);
    }
    let mut text = String::new();
    for el in p.elements() {
        if el.namespace.as_deref() == Some(ctx.w)
            && (el.local_name() == "pPr" || INLINE_MARKERS.contains(&el.local_name()))
        {
            continue;
        }
        let inline = if el.is(ctx.w, "r") {
            let run = read_run(el, ctx, stats);
            append_run_text(&run, &mut text);
            Inline::Run(run)
        } else if el.is(ctx.w, "hyperlink") {
            stats.hyperlinks += 1;
            let runs: Vec<Run> = el.elements().filter(|r| r.is(ctx.w, "r")).map(|r| read_run(r, ctx, stats)).collect();
            runs.iter().for_each(|r| append_run_text(r, &mut text));
            let url = ctx.r_attr(el, "id").and_then(|id| ctx.rel(&id)).filter(|r| r.external).map(|r| r.target.clone());
            Inline::Hyperlink { url, anchor: ctx.w_attr(el, "anchor"), runs }
        } else {
            stats.preserved_elements += 1;
            let visible = visible_text(el, ctx.w);
            text.push(' ');
            text.push_str(&visible);
            Inline::Other { kind: el.local_name().to_string(), text: visible }
        };
        paragraph.inlines.push(inline);
    }
    stats.words += count_words(&text);
    paragraph
}

fn read_paragraph_props(ppr: &Element, ctx: &ReadContext<'_>, paragraph: &mut Paragraph) {
    let w = ctx.w;
    paragraph.style_id = ppr.child(w, "pStyle").and_then(|e| ctx.w_val(e));
    paragraph.alignment = ppr.child(w, "jc").and_then(|e| ctx.w_val(e)).and_then(|v| Alignment::from_ooxml(&v));
    paragraph.section_break = ppr.child(w, "sectPr").is_some();
    if let Some(ind) = ppr.child(w, "ind") {
        let num = |names: &[&str]| names.iter().find_map(|n| ctx.w_attr(ind, n)).and_then(|v| v.parse::<i32>().ok());
        let first_line = num(&["firstLine"]).or_else(|| num(&["hanging"]).map(|h| -h));
        paragraph.indent = Some(Indent { left: num(&["left", "start"]), right: num(&["right", "end"]), first_line });
    }
    if let Some(sp) = ppr.child(w, "spacing") {
        let num = |name: &str| ctx.w_attr(sp, name).and_then(|v| v.parse::<i32>().ok());
        paragraph.spacing = Some(Spacing {
            before: num("before"),
            after: num("after"),
            line: num("line"),
            line_rule: ctx.w_attr(sp, "lineRule"),
        });
    }
    if let Some(num_pr) = ppr.child(w, "numPr") {
        let num_id = num_pr.child(w, "numId").and_then(|e| ctx.w_val(e));
        let level = num_pr.child(w, "ilvl").and_then(|e| ctx.w_val(e)).and_then(|v| v.parse().ok()).unwrap_or(0);
        if let Some(num_id) = num_id.filter(|id| id != "0") {
            paragraph.numbering = Some(Numbering { num_id, level });
        }
    }
}

fn read_run(r: &Element, ctx: &ReadContext<'_>, stats: &mut Stats) -> Run {
    let w = ctx.w;
    let mut run = Run::default();
    if let Some(rpr) = r.child(w, "rPr") {
        run.props = read_run_props(rpr, ctx);
    }
    for el in r.elements() {
        let item = if el.namespace.as_deref() != Some(w) {
            stats.preserved_elements += 1;
            RunContent::Other { kind: el.local_name().to_string() }
        } else {
            match el.local_name() {
                "rPr" | "lastRenderedPageBreak" | "softHyphen" => continue,
                "t" => {
                    push_text(&mut run.content, &el.text());
                    continue;
                }
                "noBreakHyphen" => {
                    push_text(&mut run.content, "\u{2011}");
                    continue;
                }
                "tab" | "ptab" => RunContent::Tab,
                "cr" => RunContent::LineBreak,
                "br" => match ctx.w_attr(el, "type").as_deref() {
                    Some("page") => RunContent::PageBreak,
                    Some("column") => RunContent::ColumnBreak,
                    _ => RunContent::LineBreak,
                },
                "drawing" => match read_drawing(el, ctx) {
                    Some(image) => {
                        stats.images += 1;
                        RunContent::Image(image)
                    }
                    None => {
                        stats.preserved_elements += 1;
                        RunContent::Other { kind: "drawing".into() }
                    }
                },
                other => {
                    stats.preserved_elements += 1;
                    RunContent::Other { kind: other.to_string() }
                }
            }
        };
        run.content.push(item);
    }
    run
}

fn push_text(content: &mut Vec<RunContent>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(RunContent::Text { text: prev }) = content.last_mut() {
        prev.push_str(text);
    } else {
        content.push(RunContent::Text { text: text.to_string() });
    }
}

fn read_run_props(rpr: &Element, ctx: &ReadContext<'_>) -> RunProps {
    let w = ctx.w;
    let toggle = |local: &str| rpr.child(w, local).map(|e| is_on(ctx.w_val(e).as_deref()));
    let val = |local: &str| rpr.child(w, local).and_then(|e| ctx.w_val(e));
    let strike = match (toggle("strike"), toggle("dstrike")) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), _) | (_, Some(false)) => Some(false),
        _ => None,
    };
    RunProps {
        style_id: val("rStyle"),
        bold: toggle("b"),
        italic: toggle("i"),
        underline: rpr.child(w, "u").map(|e| ctx.w_val(e).is_none_or(|v| v != "none")),
        strike,
        font: rpr
            .child(w, "rFonts")
            .and_then(|e| ["ascii", "hAnsi", "cs", "eastAsia"].iter().find_map(|a| ctx.w_attr(e, a))),
        size: val("sz").and_then(|v| v.parse().ok()),
        color: val("color").filter(|c| !c.eq_ignore_ascii_case("auto")).map(|c| c.to_ascii_uppercase()),
        highlight: val("highlight").filter(|h| h != "none"),
        vertical_align: val("vertAlign").filter(|v| v != "baseline"),
    }
}

/// OOXML on/off values: absent or true/1/on means on.
fn is_on(value: Option<&str>) -> bool {
    !matches!(value, Some("0" | "false" | "off" | "none"))
}

fn read_drawing(drawing: &Element, ctx: &ReadContext<'_>) -> Option<Image> {
    let container = drawing.elements().find(|e| e.local_name() == "inline" || e.local_name() == "anchor")?;
    let blip = find_descendant(container, "blip")?;
    let rel_id = ctx.r_attr(blip, "embed").or_else(|| ctx.r_attr(blip, "link"))?;
    let part = ctx.rel(&rel_id).filter(|r| !r.external).and_then(|r| resolve_target(ctx.main_part, &r.target));
    let extent = container.elements().find(|e| e.local_name() == "extent");
    let emu = |name: &str| extent.and_then(|e| e.attr(None, name)).and_then(|v| v.parse().ok());
    let description = container
        .elements()
        .find(|e| e.local_name() == "docPr")
        .and_then(|e| e.attr(None, "descr").or_else(|| e.attr(None, "title")))
        .filter(|d| !d.is_empty());
    Some(Image {
        rel_id,
        part,
        width_emu: emu("cx"),
        height_emu: emu("cy"),
        description,
        floating: container.local_name() == "anchor",
    })
}

fn find_descendant<'e>(el: &'e Element, local: &str) -> Option<&'e Element> {
    for child in el.elements() {
        if child.local_name() == local {
            return Some(child);
        }
        if let Some(found) = find_descendant(child, local) {
            return Some(found);
        }
    }
    None
}

fn read_table(tbl: &Element, ctx: &ReadContext<'_>, stats: &mut Stats) -> Table {
    let w = ctx.w;
    let style_id = tbl.child(w, "tblPr").and_then(|p| p.child(w, "tblStyle")).and_then(|e| ctx.w_val(e));
    let rows = tbl
        .elements()
        .filter(|e| e.is(w, "tr"))
        .map(|tr| TableRow {
            cells: tr
                .elements()
                .filter(|e| e.is(w, "tc"))
                .map(|tc| {
                    let tc_pr = tc.child(w, "tcPr");
                    let span = tc_pr
                        .and_then(|p| p.child(w, "gridSpan"))
                        .and_then(|e| ctx.w_val(e))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1);
                    let vertical_merge = tc_pr
                        .and_then(|p| p.child(w, "vMerge"))
                        .map(|e| ctx.w_val(e).unwrap_or_else(|| "continue".to_string()));
                    TableCell { span, vertical_merge, blocks: read_blocks(tc, ctx, stats) }
                })
                .collect(),
        })
        .collect();
    Table { style_id, rows }
}

fn read_section(sect: &Element, ctx: &ReadContext<'_>) -> Section {
    let w = ctx.w;
    let mut section = Section::default();
    if let Some(size) = sect.child(w, "pgSz") {
        section.page_width = ctx.w_attr(size, "w").and_then(|v| v.parse().ok());
        section.page_height = ctx.w_attr(size, "h").and_then(|v| v.parse().ok());
        section.landscape = ctx.w_attr(size, "orient").as_deref() == Some("landscape");
    }
    if let Some(margins) = sect.child(w, "pgMar") {
        let num = |name: &str| ctx.w_attr(margins, name).and_then(|v| v.parse().ok());
        section.margin_top = num("top");
        section.margin_right = num("right");
        section.margin_bottom = num("bottom");
        section.margin_left = num("left");
        section.margin_header = num("header");
        section.margin_footer = num("footer");
    }
    section.title_page = sect.child(w, "titlePg").is_some_and(|e| is_on(ctx.w_val(e).as_deref()));
    section
}

/// Visible text inside an element: `w:t` text, tabs and breaks as spaces.
pub fn visible_text(el: &Element, w: &str) -> String {
    let mut out = String::new();
    collect_text(el, w, &mut out);
    out.trim().to_string()
}

fn collect_text(el: &Element, w: &str, out: &mut String) {
    for child in el.elements() {
        if child.is(w, "t") {
            out.push_str(&child.text());
        } else if child.is(w, "tab") || child.is(w, "br") || child.is(w, "cr") {
            out.push(' ');
        } else if child.is(w, "p") {
            if !out.is_empty() {
                out.push('\n');
            }
            collect_text(child, w, out);
        } else if !child.is(w, "instrText") && !child.is(w, "delText") {
            collect_text(child, w, out);
        }
    }
}

fn append_run_text(run: &Run, out: &mut String) {
    for item in &run.content {
        match item {
            RunContent::Text { text } => out.push_str(text),
            _ => out.push(' '),
        }
    }
}

/// Counts whitespace-separated tokens containing a letter or digit (the
/// same rule as the frontend's status bar).
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().filter(|t| t.chars().any(char::is_alphanumeric)).count()
}

/// Parses the styles part.
pub fn read_styles(doc: &XmlDocument, w: &str) -> Vec<Style> {
    doc.root
        .elements()
        .filter(|e| e.is(w, "style"))
        .filter_map(|e| {
            let id = e.attr(Some(w), "styleId")?;
            let name = e.child(w, "name").and_then(|n| n.attr(Some(w), "val")).unwrap_or_else(|| id.clone());
            Some(Style {
                name,
                kind: e.attr(Some(w), "type").unwrap_or_else(|| "paragraph".to_string()),
                based_on: e.child(w, "basedOn").and_then(|b| b.attr(Some(w), "val")),
                is_default: is_on(e.attr(Some(w), "default").as_deref()) && e.attr(Some(w), "default").is_some(),
                id,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_counting_matches_frontend_rule() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("Hello, world!"), 2);
        assert_eq!(count_words("It's a well-known fact — 3.50 each"), 6);
        assert_eq!(count_words("— – -"), 0);
    }

    #[test]
    fn on_off_values() {
        for on in [None, Some("1"), Some("true"), Some("on")] {
            assert!(is_on(on), "{on:?}");
        }
        for off in [Some("0"), Some("false"), Some("off"), Some("none")] {
            assert!(!is_on(off), "{off:?}");
        }
    }
}
