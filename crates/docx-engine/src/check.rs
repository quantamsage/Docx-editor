//! Structural checks of WordprocessingML that Word is strict about.
//!
//! Word refuses (or "repairs") files whose property elements are out of
//! schema order, that repeat a property, whose table cells don't end with
//! a paragraph, or whose final section properties aren't last in the body.
//! These checks cover the containers the engine writes into; they are used
//! by the tests and to verify documents before they are saved.

use crate::props::{PPR_ORDER, RPR_ORDER, SECTPR_ORDER, TBLBORDERS_ORDER, TBLPR_ORDER, TCPR_ORDER};
use crate::xml::{Element, XmlDocument};

/// Child order of `w:tbl`, `w:tr` and `w:tc` (their leading properties).
const TBL_ORDER: &[&str] = &["tblPr", "tblGrid", "tr"];
const TR_ORDER: &[&str] = &["tblPrEx", "trPr", "tc"];
/// Elements that may repeat in an ordered container.
const REPEATABLE: &[&str] = &["headerReference", "footerReference", "tr", "tc"];

/// The WordprocessingML namespaces (transitional and strict).
pub const W_NAMESPACES: [&str; 2] = [crate::read::W_NS, crate::read::W_NS_STRICT];

/// Problems found in a WordprocessingML part (main document, header or
/// footer), as readable messages. Empty when the part is well-formed in
/// the ways checked.
pub fn problems(doc: &XmlDocument, w: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(body) = doc.root.child(w, "body") {
        let last = body.elements().last();
        if body.elements().any(|e| e.is(w, "sectPr")) && !last.is_some_and(|e| e.is(w, "sectPr")) {
            out.push("the body's section properties are not its last element".into());
        }
    }
    walk(&doc.root, w, &mut out);
    out
}

fn walk(el: &Element, w: &str, out: &mut Vec<String>) {
    let order: Option<&[&str]> = match el.local_name() {
        _ if el.namespace.as_deref() != Some(w) => None,
        "pPr" => Some(PPR_ORDER),
        "rPr" => Some(RPR_ORDER),
        "sectPr" => Some(SECTPR_ORDER),
        "tblPr" => Some(TBLPR_ORDER),
        "tcPr" => Some(TCPR_ORDER),
        "tblBorders" => Some(TBLBORDERS_ORDER),
        "tbl" => Some(TBL_ORDER),
        "tr" => Some(TR_ORDER),
        _ => None,
    };
    if let Some(order) = order {
        check_order(el, w, order, out);
    }
    if el.is(w, "tc") {
        if el.elements().position(|e| e.is(w, "tcPr")).is_some_and(|i| i != 0) {
            out.push("a table cell's properties are not its first element".into());
        }
        let blocks: Vec<&Element> =
            el.elements().filter(|e| e.is(w, "p") || e.is(w, "tbl") || e.is(w, "sdt")).collect();
        if !blocks.last().is_some_and(|e| e.is(w, "p")) {
            out.push("a table cell does not end with a paragraph".into());
        }
    }
    if el.is(w, "p") && el.elements().position(|e| e.is(w, "pPr")).is_some_and(|i| i != 0) {
        out.push("a paragraph's properties are not its first element".into());
    }
    if el.is(w, "r") && el.elements().position(|e| e.is(w, "rPr")).is_some_and(|i| i != 0) {
        out.push("a run's properties are not its first element".into());
    }
    for child in el.elements() {
        walk(child, w, out);
    }
}

fn check_order(el: &Element, w: &str, order: &[&str], out: &mut Vec<String>) {
    let rank = |local: &str| {
        // Header and footer references form one choice group.
        let local = if local == "footerReference" { "headerReference" } else { local };
        order.iter().position(|n| *n == local)
    };
    let mut previous: Option<(usize, &str)> = None;
    for child in el.elements().filter(|c| c.namespace.as_deref() == Some(w)) {
        let local = child.local_name();
        let Some(r) = rank(local) else { continue };
        if let Some((p, name)) = previous {
            if r < p {
                out.push(format!("in {}, {} comes after {}", el.name, child.name, name));
            } else if r == p && !REPEATABLE.contains(&local) {
                out.push(format!("in {}, {} appears more than once", el.name, child.name));
            }
        }
        previous = Some((r, local));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn check(body: &str) -> Vec<String> {
        let xml = format!(r#"<w:document xmlns:w="{W}" xmlns:x="urn:x"><w:body>{body}</w:body></w:document>"#);
        problems(&XmlDocument::parse(xml.as_bytes()).unwrap(), W)
    }

    #[test]
    fn accepts_valid_markup() {
        let ok = r#"<w:p><w:pPr><w:pStyle w:val="A"/><x:ext/><w:jc w:val="left"/><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="2"/></w:rPr><w:t>x</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders><w:top w:val="nil"/><w:insideV w:val="nil"/></w:tblBorders></w:tblPr><w:tblGrid/><w:tr><w:tc><w:tcPr><w:tcW w:w="1"/><w:vAlign w:val="top"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl><w:sectPr><w:headerReference/><w:footerReference/><w:headerReference/><w:pgSz/><w:pgMar/></w:sectPr>"#;
        assert_eq!(check(ok), Vec::<String>::new());
    }

    #[test]
    fn reports_order_duplicates_and_structure() {
        assert_eq!(check(r#"<w:p><w:pPr><w:jc/><w:pStyle/></w:pPr></w:p>"#), ["in w:pPr, w:pStyle comes after jc"]);
        assert_eq!(check(r#"<w:p><w:pPr><w:jc/><w:jc/></w:pPr></w:p>"#), ["in w:pPr, w:jc appears more than once"]);
        assert_eq!(check(r#"<w:sectPr><w:pgSz/><w:headerReference/></w:sectPr>"#).len(), 1);
        assert_eq!(
            check(r#"<w:tbl><w:tr><w:tc><w:p/><w:tbl/></w:tc></w:tr></w:tbl>"#),
            ["a table cell does not end with a paragraph"]
        );
        assert_eq!(check(r#"<w:sectPr/><w:p/>"#), ["the body's section properties are not its last element"]);
        assert_eq!(check(r#"<w:p><w:r><w:t/><w:rPr/></w:r></w:p>"#), ["a run's properties are not its first element"]);
    }
}
