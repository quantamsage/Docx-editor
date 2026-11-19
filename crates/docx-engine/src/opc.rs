//! Open Packaging Conventions helpers: content types and relationships.

use crate::xml::XmlDocument;

pub const CONTENT_TYPES_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub const PACKAGE_RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Relationship type suffixes (after `.../relationships/`).
pub mod rel_type {
    pub const OFFICE_DOCUMENT: &str = "officeDocument";
    pub const STYLES: &str = "styles";
    pub const NUMBERING: &str = "numbering";
    pub const HEADER: &str = "header";
    pub const FOOTER: &str = "footer";
    pub const IMAGE: &str = "image";
    pub const HYPERLINK: &str = "hyperlink";
}

/// Main document content types accepted by the editor.
pub const MAIN_CONTENT_TYPES: [&str; 4] = [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    "application/vnd.ms-word.document.macroEnabled.main+xml",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
    "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
];

/// `[Content_Types].xml`.
#[derive(Debug, Default)]
pub struct ContentTypes {
    defaults: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
}

impl ContentTypes {
    pub fn parse(doc: &XmlDocument) -> Self {
        let mut types = Self::default();
        for el in doc.root.elements() {
            if el.is(CONTENT_TYPES_NS, "Default") {
                if let (Some(ext), Some(ct)) = (el.attr(None, "Extension"), el.attr(None, "ContentType")) {
                    types.defaults.push((ext.to_ascii_lowercase(), ct));
                }
            } else if el.is(CONTENT_TYPES_NS, "Override")
                && let (Some(part), Some(ct)) = (el.attr(None, "PartName"), el.attr(None, "ContentType"))
            {
                types.overrides.push((part.trim_start_matches('/').to_ascii_lowercase(), ct));
            }
        }
        types
    }

    /// The content type of a part (name without a leading slash).
    pub fn of(&self, part: &str) -> Option<&str> {
        let part = part.trim_start_matches('/').to_ascii_lowercase();
        if let Some((_, ct)) = self.overrides.iter().find(|(name, _)| *name == part) {
            return Some(ct);
        }
        let ext = part.rsplit_once('.').map(|(_, e)| e)?;
        self.defaults.iter().find(|(e, _)| e == ext).map(|(_, ct)| ct.as_str())
    }
}

/// One relationship from a `.rels` part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    /// Full type URI.
    pub rel_type: String,
    /// Target as written (relative to the source part's folder, or a URL).
    pub target: String,
    /// `TargetMode="External"` (e.g. web links).
    pub external: bool,
}

impl Relationship {
    /// Whether the type URI ends with `/<suffix>` (works for both the
    /// transitional and strict OOXML namespaces).
    pub fn is_type(&self, suffix: &str) -> bool {
        self.rel_type.rsplit('/').next() == Some(suffix)
    }
}

/// Parses a `.rels` part.
pub fn parse_relationships(doc: &XmlDocument) -> Vec<Relationship> {
    doc.root
        .elements()
        .filter(|el| el.is(PACKAGE_RELS_NS, "Relationship"))
        .filter_map(|el| {
            Some(Relationship {
                id: el.attr(None, "Id")?,
                rel_type: el.attr(None, "Type")?,
                target: el.attr(None, "Target")?,
                external: el.attr(None, "TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External")),
            })
        })
        .collect()
}

/// The `.rels` part name for a source part: `word/document.xml` →
/// `word/_rels/document.xml.rels`; the package root (`""`) → `_rels/.rels`.
pub fn rels_part_for(source: &str) -> String {
    match source.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None if source.is_empty() => "_rels/.rels".to_string(),
        None => format!("_rels/{source}.rels"),
    }
}

/// Resolves an internal relationship target against the source part,
/// returning a part name without a leading slash, or `None` if it would
/// leave the package.
pub fn resolve_target(source: &str, target: &str) -> Option<String> {
    let target = target.split('#').next().unwrap_or("");
    let mut segments: Vec<&str> = if let Some(absolute) = target.strip_prefix('/') {
        return normalize(absolute.split('/').collect());
    } else {
        source.rsplit_once('/').map(|(dir, _)| dir.split('/').collect()).unwrap_or_default()
    };
    segments.extend(target.split('/'));
    normalize(segments)
}

fn normalize(segments: Vec<&str>) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in segments {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    if out.is_empty() { None } else { Some(out.join("/")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_prefer_overrides() {
        let xml = format!(
            r#"<Types xmlns="{CONTENT_TYPES_NS}"><Default Extension="XML" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="{}"/></Types>"#,
            MAIN_CONTENT_TYPES[0]
        );
        let types = ContentTypes::parse(&XmlDocument::parse(xml.as_bytes()).unwrap());
        assert_eq!(types.of("word/document.xml"), Some(MAIN_CONTENT_TYPES[0]));
        assert_eq!(types.of("/Word/Document.xml"), Some(MAIN_CONTENT_TYPES[0]));
        assert_eq!(types.of("word/styles.xml"), Some("application/xml"));
        assert_eq!(types.of("word/media/a.PNG"), Some("image/png"));
        assert_eq!(types.of("word/media/a.gif"), None);
    }

    #[test]
    fn relationships_and_targets() {
        let xml = format!(
            r#"<Relationships xmlns="{PACKAGE_RELS_NS}"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/hyperlink" Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/></Relationships>"#
        );
        let rels = parse_relationships(&XmlDocument::parse(xml.as_bytes()).unwrap());
        assert_eq!(rels.len(), 2);
        assert!(rels[0].is_type(rel_type::STYLES) && !rels[0].external);
        assert!(rels[1].is_type(rel_type::HYPERLINK) && rels[1].external);
        assert_eq!(rels[1].target, "https://example.com/?a=1&b=2");

        assert_eq!(rels_part_for("word/document.xml"), "word/_rels/document.xml.rels");
        assert_eq!(rels_part_for(""), "_rels/.rels");
        assert_eq!(resolve_target("word/document.xml", "media/image1.png").as_deref(), Some("word/media/image1.png"));
        assert_eq!(
            resolve_target("word/document.xml", "../customXml/item1.xml").as_deref(),
            Some("customXml/item1.xml")
        );
        assert_eq!(resolve_target("", "word/document.xml").as_deref(), Some("word/document.xml"));
        assert_eq!(resolve_target("word/document.xml", "/word/styles.xml").as_deref(), Some("word/styles.xml"));
        assert_eq!(resolve_target("word/document.xml", "../../../etc/passwd"), None);
    }
}
