//! Setting and removing OOXML property elements in schema order.
//!
//! Property containers such as `w:pPr` (and later `w:rPr`) are XML Schema sequences:
//! Word rejects a file whose children are out of order. New properties are
//! therefore inserted before the first existing sibling that must follow
//! them. Siblings the engine does not know (extensions, alternate content)
//! are never moved or removed.

use crate::xml::{Element, Node};

/// Child order of `w:pPr` (CT_PPr).
pub const PPR_ORDER: &[&str] = &[
    "pStyle",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "framePr",
    "widowControl",
    "numPr",
    "suppressLineNumbers",
    "pBdr",
    "shd",
    "tabs",
    "suppressAutoHyphens",
    "kinsoku",
    "wordWrap",
    "overflowPunct",
    "topLinePunct",
    "autoSpaceDE",
    "autoSpaceDN",
    "bidi",
    "adjustRightInd",
    "snapToGrid",
    "spacing",
    "ind",
    "contextualSpacing",
    "mirrorIndents",
    "suppressOverlap",
    "jc",
    "textDirection",
    "textAlignment",
    "textboxTightWrap",
    "outlineLvl",
    "divId",
    "cnfStyle",
    "rPr",
    "sectPr",
    "pPrChange",
];

/// Child order of `w:rPr` (CT_RPr: EG_RPrBase, then the change record).
pub const RPR_ORDER: &[&str] = &[
    "rStyle",
    "rFonts",
    "b",
    "bCs",
    "i",
    "iCs",
    "caps",
    "smallCaps",
    "strike",
    "dstrike",
    "outline",
    "shadow",
    "emboss",
    "imprint",
    "noProof",
    "snapToGrid",
    "vanish",
    "webHidden",
    "color",
    "spacing",
    "w",
    "kern",
    "position",
    "sz",
    "szCs",
    "highlight",
    "u",
    "effect",
    "bdr",
    "shd",
    "fitText",
    "vertAlign",
    "rtl",
    "cs",
    "em",
    "lang",
    "eastAsianLayout",
    "specVanish",
    "oMath",
    "rPrChange",
];

/// Names used to create elements in the document's WordprocessingML
/// namespace with the prefix it already declares.
///
/// Attributes always need a prefix: unprefixed attributes are in no
/// namespace, even when WordprocessingML is the default namespace.
#[derive(Debug, Clone)]
pub struct WNames {
    pub ns: String,
    /// Prefix for elements (may be empty when it is the default namespace).
    pub prefix: String,
    /// Prefix for attributes (never empty).
    pub attr_prefix: String,
}

impl WNames {
    pub fn element(&self, local: &str) -> Element {
        Element::new(&self.prefix, local, Some(&self.ns))
    }

    pub fn attr_name(&self, local: &str) -> String {
        format!("{}:{local}", self.attr_prefix)
    }

    /// Sets a `w:`-namespaced attribute.
    pub fn set_attr(&self, el: &mut Element, local: &str, value: &str) {
        el.set_attr(&self.attr_name(local), Some(&self.ns), value);
    }
}

fn rank(order: &[&str], local: &str) -> Option<usize> {
    order.iter().position(|name| *name == local)
}

/// Returns the property child `local` of `parent`, creating it at its
/// schema position if missing.
pub fn ensure_child<'a>(parent: &'a mut Element, w: &WNames, order: &[&str], local: &str) -> &'a mut Element {
    let existing = parent.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(&w.ns, local)));
    let index = match existing {
        Some(index) => index,
        None => {
            let target = rank(order, local).expect("property must be listed in its schema order");
            let insert_at = parent
                .children
                .iter()
                .position(|n| match n {
                    Node::Element(e) if e.namespace.as_deref() == Some(w.ns.as_str()) => {
                        rank(order, e.local_name()).is_some_and(|r| r > target)
                    }
                    _ => false,
                })
                .unwrap_or(parent.children.len());
            parent.children.insert(insert_at, Node::Element(w.element(local)));
            insert_at
        }
    };
    match &mut parent.children[index] {
        Node::Element(e) => e,
        _ => unreachable!("index points at an element"),
    }
}

/// Puts the known children of `parent` into schema order (stable). Unknown
/// elements and other nodes stay directly after the known element they
/// followed, so extensions keep their place relative to their neighbours.
pub fn sort_schema(parent: &mut Element, w: &WNames, order: &[&str]) {
    let mut current = 0usize;
    let mut keyed: Vec<(usize, usize, Node)> = std::mem::take(&mut parent.children)
        .into_iter()
        .enumerate()
        .map(|(i, node)| {
            if let Node::Element(e) = &node
                && e.namespace.as_deref() == Some(w.ns.as_str())
                && let Some(r) = rank(order, e.local_name())
            {
                current = r;
            }
            (current, i, node)
        })
        .collect();
    keyed.sort_by_key(|(r, i, _)| (*r, *i));
    parent.children = keyed.into_iter().map(|(_, _, n)| n).collect();
}

/// Removes every `local` child of `parent`; returns whether any existed.
pub fn remove_child(parent: &mut Element, w: &WNames, local: &str) -> bool {
    let before = parent.children.len();
    parent.children.retain(|n| !matches!(n, Node::Element(e) if e.is(&w.ns, local)));
    parent.children.len() != before
}

/// Returns the paragraph's `w:pPr`, creating it as the first child.
pub fn ensure_ppr<'a>(p: &'a mut Element, w: &WNames) -> &'a mut Element {
    let existing = p.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(&w.ns, "pPr")));
    let index = existing.unwrap_or_else(|| {
        p.children.insert(0, Node::Element(w.element("pPr")));
        0
    });
    match &mut p.children[index] {
        Node::Element(e) => e,
        _ => unreachable!("index points at an element"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::XmlDocument;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn names() -> WNames {
        WNames { ns: W.into(), prefix: "w".into(), attr_prefix: "w".into() }
    }

    fn ppr(inner: &str) -> Element {
        let xml = format!("<w:pPr xmlns:w=\"{W}\" xmlns:x=\"urn:x\">{inner}</w:pPr>");
        XmlDocument::parse(xml.as_bytes()).unwrap().root
    }

    fn children(el: &Element) -> Vec<String> {
        el.elements().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn inserts_in_schema_order() {
        let w = names();
        let mut el = ppr("<w:keepNext/><w:spacing w:after=\"0\"/><x:ext/><w:rPr/>");
        w.set_attr(ensure_child(&mut el, &w, PPR_ORDER, "jc"), "val", "center");
        w.set_attr(ensure_child(&mut el, &w, PPR_ORDER, "pStyle"), "val", "Heading1");
        ensure_child(&mut el, &w, PPR_ORDER, "ind");
        // New properties go right before the next known sibling that must follow them.
        assert_eq!(children(&el), ["w:pStyle", "w:keepNext", "w:spacing", "x:ext", "w:ind", "w:jc", "w:rPr"]);

        // Existing children are reused, not duplicated.
        w.set_attr(ensure_child(&mut el, &w, PPR_ORDER, "jc"), "val", "right");
        assert_eq!(el.elements().filter(|e| e.is(W, "jc")).count(), 1);
        assert_eq!(el.child(W, "jc").unwrap().attr(Some(W), "val").as_deref(), Some("right"));
    }

    #[test]
    fn appends_when_nothing_follows() {
        let w = names();
        let mut el = ppr("<w:pStyle w:val=\"A\"/>");
        ensure_child(&mut el, &w, PPR_ORDER, "jc");
        assert_eq!(children(&el), ["w:pStyle", "w:jc"]);
    }

    #[test]
    fn removes_children() {
        let w = names();
        let mut el = ppr("<w:pStyle w:val=\"A\"/><w:jc w:val=\"left\"/>");
        assert!(remove_child(&mut el, &w, "jc"));
        assert!(!remove_child(&mut el, &w, "jc"));
        assert_eq!(children(&el), ["w:pStyle"]);
    }

    #[test]
    fn uses_the_document_prefix() {
        let w = WNames { ns: W.into(), prefix: String::new(), attr_prefix: "w".into() };
        let mut el = Element::new("", "pPr", Some(W));
        w.set_attr(ensure_child(&mut el, &w, PPR_ORDER, "jc"), "val", "both");
        let mut out = String::new();
        el.write_to(&mut out);
        assert_eq!(out, "<pPr><jc w:val=\"both\"/></pPr>");
    }

    #[test]
    fn order_list_has_no_duplicates() {
        {
            let order = PPR_ORDER;
            let mut seen = std::collections::HashSet::new();
            assert!(order.iter().all(|n| seen.insert(*n)));
        }
    }
}
