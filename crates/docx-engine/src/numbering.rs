//! List numbering (`word/numbering.xml`): reading definitions for display,
//! and finding or creating the bullet and numbered list definitions that
//! the editor applies.
//!
//! A paragraph is in a list when its `w:numPr` names a `w:num`, which points
//! to a `w:abstractNum` defining the marker of each of 9 levels. To avoid
//! piling up definitions, the editor reuses the first existing list whose
//! first level is a bullet (for bullets) or decimal (for numbers); only when
//! the document has none does it add one.

use serde::Serialize;

use crate::format::ListKind;
use crate::xml::{Element, Node, XmlDocument};

pub const NUMBERING_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
pub const NUMBERING_REL_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";

/// One level of a list, as the editor needs to draw its marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListLevel {
    /// `w:numFmt`, e.g. `bullet`, `decimal`, `lowerLetter`.
    pub format: String,
    /// `w:lvlText`, e.g. `•` or `%1.`.
    pub text: String,
    pub start: u32,
    /// Left indent in twips, if the level sets one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_left: Option<i32>,
}

/// A numbering instance (`w:num`) with its levels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListDefinition {
    pub num_id: String,
    pub levels: Vec<ListLevel>,
}

/// An empty numbering part.
pub fn empty_part(w_ns: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="{w_ns}"></w:numbering>"#
    )
    .into_bytes()
}

fn attr(el: &Element, w: &str, local: &str) -> Option<String> {
    el.attr(Some(w), local)
}

fn child_val(el: &Element, w: &str, local: &str) -> Option<String> {
    el.child(w, local).and_then(|c| attr(c, w, "val"))
}

fn read_levels(abs: &Element, w: &str) -> Vec<ListLevel> {
    let mut levels: Vec<(u8, ListLevel)> = abs
        .elements()
        .filter(|e| e.is(w, "lvl"))
        .filter_map(|lvl| {
            let ilvl = attr(lvl, w, "ilvl")?.parse().ok()?;
            Some((
                ilvl,
                ListLevel {
                    format: child_val(lvl, w, "numFmt").unwrap_or_else(|| "decimal".into()),
                    text: child_val(lvl, w, "lvlText").unwrap_or_default(),
                    start: child_val(lvl, w, "start").and_then(|s| s.parse().ok()).unwrap_or(1),
                    indent_left: lvl
                        .child(w, "pPr")
                        .and_then(|p| p.child(w, "ind"))
                        .and_then(|ind| attr(ind, w, "left").or_else(|| attr(ind, w, "start")))
                        .and_then(|v| v.parse().ok()),
                },
            ))
        })
        .collect();
    levels.sort_by_key(|(i, _)| *i);
    levels.into_iter().map(|(_, l)| l).collect()
}

/// All list definitions in a numbering part.
pub fn read(doc: &XmlDocument, w: &str) -> Vec<ListDefinition> {
    let root = &doc.root;
    root.elements()
        .filter(|e| e.is(w, "num"))
        .filter_map(|num| {
            let num_id = attr(num, w, "numId")?;
            let abs_id = child_val(num, w, "abstractNumId")?;
            let abs = root
                .elements()
                .find(|e| e.is(w, "abstractNum") && attr(e, w, "abstractNumId").as_deref() == Some(&abs_id))?;
            Some(ListDefinition { num_id, levels: read_levels(abs, w) })
        })
        .collect()
}

fn max_id(root: &Element, w: &str, local: &str, id_attr: &str) -> i64 {
    root.elements()
        .filter(|e| e.is(w, local))
        .filter_map(|e| attr(e, w, id_attr)?.parse::<i64>().ok())
        .max()
        .unwrap_or(0)
}

/// Returns the numId of a list of the given kind, adding the definition to
/// the numbering part if the document has none.
pub fn find_or_create(doc: &mut XmlDocument, w: &str, prefix: &str, kind: ListKind) -> String {
    let wanted = match kind {
        ListKind::Bullet => "bullet",
        ListKind::Number => "decimal",
    };
    if let Some(def) = read(doc, w).into_iter().find(|d| d.levels.first().is_some_and(|l| l.format == wanted)) {
        return def.num_id;
    }

    let abs_id = (max_id(&doc.root, w, "abstractNum", "abstractNumId") + 1).max(1);
    let num_id = (max_id(&doc.root, w, "num", "numId") + 1).max(1);
    let p = if prefix.is_empty() { String::new() } else { format!("{prefix}:") };
    let mut levels = String::new();
    for ilvl in 0..9 {
        let (fmt, text) = match kind {
            ListKind::Bullet => ("bullet", ["•", "◦", "▪"][ilvl % 3].to_string()),
            ListKind::Number => (["decimal", "lowerLetter", "lowerRoman"][ilvl % 3], format!("%{}.", ilvl + 1)),
        };
        let left = 720 * (ilvl + 1);
        levels.push_str(&format!(
            r#"<{p}lvl {p}ilvl="{ilvl}"><{p}start {p}val="1"/><{p}numFmt {p}val="{fmt}"/><{p}lvlText {p}val="{text}"/><{p}lvlJc {p}val="left"/><{p}pPr><{p}ind {p}left="{left}" {p}hanging="360"/></{p}pPr></{p}lvl>"#
        ));
    }
    let fragment = format!(
        r#"<{p}abstractNum {p}abstractNumId="{abs_id}"><{p}multiLevelType {p}val="hybridMultilevel"/>{levels}</{p}abstractNum><{p}num {p}numId="{num_id}"><{p}abstractNumId {p}val="{abs_id}"/></{p}num>"#
    );
    let mut new = parse_fragment(prefix, w, &fragment);
    let num = new.pop().expect("fragment has a num");
    let abs = new.pop().expect("fragment has an abstractNum");

    // Schema order: every abstractNum comes before the first num.
    let children = &mut doc.root.children;
    let first_num = children.iter().position(|n| matches!(n, Node::Element(e) if e.is(w, "num")));
    match first_num {
        Some(i) => children.insert(i, Node::Element(abs)),
        None => {
            let last_abs = children.iter().rposition(|n| matches!(n, Node::Element(e) if e.is(w, "abstractNum")));
            children.insert(last_abs.map_or(0, |i| i + 1), Node::Element(abs));
        }
    }
    let last_num =
        children.iter().rposition(|n| matches!(n, Node::Element(e) if e.is(w, "num") || e.is(w, "abstractNum")));
    children.insert(last_num.map_or(children.len(), |i| i + 1), Node::Element(num));
    num_id.to_string()
}

/// Parses elements written with `prefix` bound to namespace `ns`.
pub fn parse_fragment(prefix: &str, ns: &str, xml: &str) -> Vec<Element> {
    let wrapper = if prefix.is_empty() {
        format!(r#"<fragment xmlns="{ns}">{xml}</fragment>"#)
    } else {
        format!(r#"<{prefix}:fragment xmlns:{prefix}="{ns}">{xml}</{prefix}:fragment>"#)
    };
    let doc = XmlDocument::parse(wrapper.as_bytes()).expect("built-in XML fragments are well formed");
    doc.root
        .children
        .into_iter()
        .filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    #[test]
    fn creates_and_then_reuses_definitions() {
        let mut doc = XmlDocument::parse(&empty_part(W)).unwrap();
        let bullet = find_or_create(&mut doc, W, "w", ListKind::Bullet);
        let number = find_or_create(&mut doc, W, "w", ListKind::Number);
        assert_eq!((bullet.as_str(), number.as_str()), ("1", "2"));
        assert_eq!(find_or_create(&mut doc, W, "w", ListKind::Bullet), "1", "reused");

        let defs = read(&doc, W);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].levels.len(), 9);
        assert_eq!((defs[0].levels[0].format.as_str(), defs[0].levels[0].text.as_str()), ("bullet", "•"));
        assert_eq!((defs[1].levels[1].format.as_str(), defs[1].levels[1].text.as_str()), ("lowerLetter", "%2."));
        assert_eq!(defs[1].levels[0].indent_left, Some(720));

        // Schema order: abstractNums before nums.
        let order: Vec<&str> = doc.root.elements().map(|e| e.local_name()).collect();
        assert_eq!(order, ["abstractNum", "abstractNum", "num", "num"]);
        let xml = String::from_utf8(doc.to_bytes()).unwrap();
        assert!(XmlDocument::parse(xml.as_bytes()).is_ok());
    }

    #[test]
    fn reuses_a_documents_own_list() {
        let xml = format!(
            r#"<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="7"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1)"/></w:lvl></w:abstractNum><w:num w:numId="12"><w:abstractNumId w:val="7"/></w:num></w:numbering>"#
        );
        let mut doc = XmlDocument::parse(xml.as_bytes()).unwrap();
        assert_eq!(find_or_create(&mut doc, W, "w", ListKind::Number), "12");
        assert_eq!(find_or_create(&mut doc, W, "w", ListKind::Bullet), "13");
        assert!(String::from_utf8(doc.to_bytes()).unwrap().contains(r#"<w:abstractNum w:abstractNumId="8">"#));
    }
}
