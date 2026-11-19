//! A lossless XML tree.
//!
//! Parsing and writing a document without changing it reproduces the input
//! exactly: every element keeps the raw text of its start tag (attribute
//! order, quoting, whitespace and namespace declarations), text keeps its
//! original escaping, and comments, processing instructions and the XML
//! declaration are kept verbatim. Only elements that are modified through
//! the mutation API are re-serialized from their parsed form.
//!
//! Element and attribute names are resolved against their namespace
//! declarations, so lookups work whatever prefix a document uses.
//!
//! DOCTYPE declarations are rejected: OOXML never uses them, and refusing
//! them rules out entity-expansion attacks.

use quick_xml::escape::{escape, unescape};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

/// Namespace of the XML `xml:` prefix.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// A node in the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    /// Character data, stored in its original escaped form.
    Text(String),
    /// `<![CDATA[...]]>` content.
    CData(String),
    /// `<!--...-->` content.
    Comment(String),
    /// `<?...?>` content (processing instruction or XML declaration).
    Instruction(String),
}

/// An attribute with its value in escaped form.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// Qualified name as written (e.g. `w:val`).
    pub name: String,
    /// Resolved namespace URI (`None` for unprefixed attributes).
    pub namespace: Option<String>,
    raw_value: String,
}

impl Attribute {
    /// The unescaped value.
    pub fn value(&self) -> String {
        unescape(&self.raw_value).map(|v| v.into_owned()).unwrap_or_else(|_| self.raw_value.clone())
    }

    fn local_name(&self) -> &str {
        local_part(&self.name)
    }
}

/// An element.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Qualified name as written (e.g. `w:p`).
    pub name: String,
    /// Resolved namespace URI.
    pub namespace: Option<String>,
    attributes: Vec<Attribute>,
    /// The exact original text between `<` and `>` (without a trailing `/`
    /// for empty elements). Cleared when attributes change.
    raw_start: Option<String>,
    /// Written as `<x/>` when it has no children.
    self_closing: bool,
    pub children: Vec<Node>,
}

impl Element {
    /// Creates an element. `prefix` is the namespace prefix to write
    /// (`""` for none); the namespace must already be declared in scope.
    pub fn new(prefix: &str, local: &str, namespace: Option<&str>) -> Self {
        let name = if prefix.is_empty() { local.to_string() } else { format!("{prefix}:{local}") };
        Self {
            name,
            namespace: namespace.map(str::to_string),
            attributes: Vec::new(),
            raw_start: None,
            self_closing: true,
            children: Vec::new(),
        }
    }

    /// The local part of the name (`p` for `w:p`).
    pub fn local_name(&self) -> &str {
        local_part(&self.name)
    }

    /// Whether this element has the given namespace and local name.
    pub fn is(&self, namespace: &str, local: &str) -> bool {
        self.local_name() == local && self.namespace.as_deref() == Some(namespace)
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// The unescaped value of the attribute with this namespace and local
    /// name. Pass `None` for unprefixed attributes.
    pub fn attr(&self, namespace: Option<&str>, local: &str) -> Option<String> {
        self.attributes
            .iter()
            .find(|a| a.local_name() == local && a.namespace.as_deref() == namespace)
            .map(Attribute::value)
    }

    /// Sets (or adds) an attribute. `name` is the qualified name to write
    /// if the attribute is new.
    pub fn set_attr(&mut self, name: &str, namespace: Option<&str>, value: &str) {
        let raw_value = escape(value).into_owned();
        let local = local_part(name);
        match self.attributes.iter_mut().find(|a| a.local_name() == local && a.namespace.as_deref() == namespace) {
            Some(attr) if attr.raw_value == raw_value => return,
            Some(attr) => attr.raw_value = raw_value,
            None => self.attributes.push(Attribute {
                name: name.to_string(),
                namespace: namespace.map(str::to_string),
                raw_value,
            }),
        }
        self.raw_start = None;
    }

    /// Removes an attribute; returns whether it existed.
    pub fn remove_attr(&mut self, namespace: Option<&str>, local: &str) -> bool {
        let before = self.attributes.len();
        self.attributes.retain(|a| !(a.local_name() == local && a.namespace.as_deref() == namespace));
        let removed = self.attributes.len() != before;
        if removed {
            self.raw_start = None;
        }
        removed
    }

    /// Child elements, skipping text, comments and other nodes.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// Mutable child elements.
    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// The first child element with this name.
    pub fn child(&self, namespace: &str, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.is(namespace, local))
    }

    /// The first child element with this name, mutably.
    pub fn child_mut(&mut self, namespace: &str, local: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.is(namespace, local))
    }

    /// The unescaped text of this element's direct text and CDATA children.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for node in &self.children {
            match node {
                Node::Text(raw) => out.push_str(&unescape(raw).map(|t| t.into_owned()).unwrap_or_else(|_| raw.clone())),
                Node::CData(raw) => out.push_str(raw),
                _ => {}
            }
        }
        out
    }

    /// Replaces all children with one escaped text node.
    pub fn set_text(&mut self, text: &str) {
        self.children.clear();
        if !text.is_empty() {
            self.children.push(Node::Text(escape_text(text)));
        }
    }

    /// Serializes this element and its descendants.
    pub fn write_to(&self, out: &mut String) {
        out.push('<');
        match &self.raw_start {
            Some(raw) => out.push_str(raw),
            None => {
                out.push_str(&self.name);
                for attr in &self.attributes {
                    out.push(' ');
                    out.push_str(&attr.name);
                    out.push_str("=\"");
                    // Raw values may come from single-quoted originals and
                    // contain a bare double quote.
                    out.push_str(&attr.raw_value.replace('"', "&quot;"));
                    out.push('"');
                }
            }
        }
        if self.children.is_empty() && self.self_closing {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for child in &self.children {
            write_node(child, out);
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

/// A parsed XML document.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlDocument {
    /// Nodes before the root element (declaration, comments, whitespace).
    pub prolog: Vec<Node>,
    pub root: Element,
    /// Nodes after the root element.
    pub epilog: Vec<Node>,
}

/// Why a document could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError(pub String);

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl XmlDocument {
    /// Parses UTF-8 XML (an optional byte-order mark is kept).
    pub fn parse(bytes: &[u8]) -> Result<Self, XmlError> {
        let (bom, body) = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
            Some(rest) => (true, rest),
            None => (false, bytes),
        };
        if body.starts_with(&[0xFF, 0xFE]) || body.starts_with(&[0xFE, 0xFF]) {
            return Err(XmlError("UTF-16 encoded XML is not supported".into()));
        }
        let text = std::str::from_utf8(body).map_err(|_| XmlError("the XML is not valid UTF-8".into()))?;
        let mut doc = Parser::new(text).parse()?;
        if bom {
            doc.prolog.insert(0, Node::Text("\u{FEFF}".into()));
        }
        Ok(doc)
    }

    /// Serializes the document.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        for node in &self.prolog {
            write_node(node, &mut out);
        }
        self.root.write_to(&mut out);
        for node in &self.epilog {
            write_node(node, &mut out);
        }
        out.into_bytes()
    }

    /// The prefix declared for `namespace` on the root element, if any
    /// (`Some("")` when it is the default namespace).
    pub fn prefix_for(&self, namespace: &str) -> Option<String> {
        self.root.attributes.iter().find_map(|a| {
            if a.value() != namespace {
                return None;
            }
            if a.name == "xmlns" { Some(String::new()) } else { a.name.strip_prefix("xmlns:").map(str::to_string) }
        })
    }
}

fn write_node(node: &Node, out: &mut String) {
    match node {
        Node::Element(e) => e.write_to(out),
        Node::Text(raw) => out.push_str(raw),
        Node::CData(raw) => {
            out.push_str("<![CDATA[");
            out.push_str(raw);
            out.push_str("]]>");
        }
        Node::Comment(raw) => {
            out.push_str("<!--");
            out.push_str(raw);
            out.push_str("-->");
        }
        Node::Instruction(raw) => {
            out.push_str("<?");
            out.push_str(raw);
            out.push_str("?>");
        }
    }
}

/// Escapes text content (`&`, `<`, `>`); quotes are left alone, as Word does.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn local_part(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn prefix_part(name: &str) -> &str {
    name.split_once(':').map_or("", |(prefix, _)| prefix)
}

struct Parser<'a> {
    reader: Reader<&'a [u8]>,
    /// Namespace bindings per open element: (prefix, uri).
    scopes: Vec<Vec<(String, String)>>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        let mut reader = Reader::from_str(text);
        let config = reader.config_mut();
        config.trim_text(false);
        config.check_end_names = true;
        config.expand_empty_elements = false;
        Self { reader, scopes: Vec::new() }
    }

    fn err(&self, message: impl std::fmt::Display) -> XmlError {
        XmlError(format!("{message} (at byte {})", self.reader.buffer_position()))
    }

    fn parse(mut self) -> Result<XmlDocument, XmlError> {
        let mut prolog = Vec::new();
        let mut epilog = Vec::new();
        let mut root: Option<Element> = None;
        // Open elements; the last is the current parent.
        let mut stack: Vec<Element> = Vec::new();

        loop {
            let event = self.reader.read_event().map_err(|e| self.err(e))?;
            let node = match event {
                Event::Eof => break,
                Event::DocType(_) => return Err(self.err("DOCTYPE declarations are not allowed")),
                Event::Start(start) => {
                    let element = self.start_element(&start, false)?;
                    stack.push(element);
                    continue;
                }
                Event::Empty(start) => {
                    let element = self.start_element(&start, true)?;
                    self.scopes.pop();
                    Node::Element(element)
                }
                Event::End(_) => {
                    let element = stack.pop().ok_or_else(|| self.err("unexpected end tag"))?;
                    self.scopes.pop();
                    Node::Element(element)
                }
                Event::Text(text) => Node::Text(text.to_string()),
                Event::GeneralRef(reference) => Node::Text(format!("&{};", &*reference)),
                Event::CData(data) => Node::CData(data.to_string()),
                Event::Comment(comment) => Node::Comment(comment.to_string()),
                Event::Decl(decl) => Node::Instruction(decl.to_string()),
                Event::PI(pi) => Node::Instruction(pi.to_string()),
            };

            match stack.last_mut() {
                Some(parent) => push_merging_text(&mut parent.children, node),
                None => match node {
                    Node::Element(element) => {
                        if root.is_some() {
                            return Err(self.err("more than one root element"));
                        }
                        root = Some(element);
                    }
                    Node::Text(ref raw) if !raw.trim().is_empty() && raw != "\u{FEFF}" => {
                        return Err(self.err("text outside the root element"));
                    }
                    other if root.is_none() => push_merging_text(&mut prolog, other),
                    other => push_merging_text(&mut epilog, other),
                },
            }
        }
        if !stack.is_empty() {
            return Err(self.err("unclosed element"));
        }
        let root = root.ok_or_else(|| XmlError("the document has no root element".into()))?;
        Ok(XmlDocument { prolog, root, epilog })
    }

    fn start_element(&mut self, start: &BytesStart<'_>, empty: bool) -> Result<Element, XmlError> {
        let raw_start = start.to_string();
        let name = start.name().as_ref().to_string();

        let mut raw_attrs: Vec<(String, String)> = Vec::new();
        let mut bindings: Vec<(String, String)> = Vec::new();
        for attr in start.attributes().with_checks(true) {
            let attr = attr.map_err(|e| self.err(e))?;
            let key = attr.key.as_ref().to_string();
            let raw_value = attr.value.to_string();
            if key == "xmlns" || key.starts_with("xmlns:") {
                let uri = unescape(&raw_value).map_err(|e| self.err(e))?.into_owned();
                bindings.push((key.strip_prefix("xmlns:").unwrap_or("").to_string(), uri));
            }
            raw_attrs.push((key, raw_value));
        }
        self.scopes.push(bindings);

        let namespace = self.resolve(prefix_part(&name), true);
        let attributes = raw_attrs
            .into_iter()
            .map(|(key, raw_value)| {
                let namespace = if key == "xmlns" || key.starts_with("xmlns:") {
                    None
                } else {
                    let prefix = prefix_part(&key);
                    // Unprefixed attributes are in no namespace.
                    if prefix.is_empty() { None } else { self.resolve(prefix, false) }
                };
                Attribute { name: key, namespace, raw_value }
            })
            .collect();

        Ok(Element {
            name,
            namespace,
            attributes,
            raw_start: Some(raw_start),
            self_closing: empty,
            children: Vec::new(),
        })
    }

    fn resolve(&self, prefix: &str, use_default: bool) -> Option<String> {
        if prefix == "xml" {
            return Some(XML_NS.to_string());
        }
        if prefix.is_empty() && !use_default {
            return None;
        }
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|(p, _)| p == prefix)
            .map(|(_, uri)| uri.clone())
            .filter(|uri| !uri.is_empty())
    }
}

/// Appends a node, merging adjacent text so that `a&amp;b` stays one node.
fn push_merging_text(nodes: &mut Vec<Node>, node: Node) {
    if let (Some(Node::Text(prev)), Node::Text(next)) = (nodes.last_mut(), &node) {
        prev.push_str(next);
        return;
    }
    nodes.push(node);
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    const SAMPLE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<!-- leading comment -->\n\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006'  mc:Ignorable=\"w14\">\
<w:body>\n  <w:p w:rsidR=\"00AB\"><w:r><w:t xml:space=\"preserve\"> A &amp; B &lt;tag&gt; &#169; \"q\" </w:t></w:r></w:p>\
<w:p/><w:p ></w:p><w:p w:x = 'single \"quoted\"' />\
<?mso-application progid=\"Word.Document\"?><x:custom xmlns:x=\"urn:x\"><![CDATA[<raw> & stuff]]></x:custom>\
<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr></w:body></w:document>\n<!-- trailing -->";

    #[test]
    fn round_trip_is_byte_exact() {
        let doc = XmlDocument::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(doc.to_bytes()).unwrap(), SAMPLE);

        let with_bom = [b"\xEF\xBB\xBF".as_slice(), SAMPLE.as_bytes()].concat();
        assert_eq!(XmlDocument::parse(&with_bom).unwrap().to_bytes(), with_bom);
    }

    #[test]
    fn resolves_namespaces_and_reads_text() {
        let doc = XmlDocument::parse(SAMPLE.as_bytes()).unwrap();
        assert!(doc.root.is(W, "document"));
        let body = doc.root.child(W, "body").unwrap();
        let paragraphs: Vec<_> = body.elements().filter(|e| e.is(W, "p")).collect();
        assert_eq!(paragraphs.len(), 4);
        assert_eq!(paragraphs[0].attr(Some(W), "rsidR").as_deref(), Some("00AB"));
        assert_eq!(paragraphs[3].attr(Some(W), "x").as_deref(), Some("single \"quoted\""));
        let t = paragraphs[0].child(W, "r").unwrap().child(W, "t").unwrap();
        assert_eq!(t.text(), " A & B <tag> © \"q\" ");
        assert_eq!(t.attr(Some(XML_NS), "space").as_deref(), Some("preserve"));
        let custom = body.elements().find(|e| e.local_name() == "custom").unwrap();
        assert_eq!(custom.namespace.as_deref(), Some("urn:x"));
        assert_eq!(custom.text(), "<raw> & stuff");
        assert_eq!(doc.prefix_for(W).as_deref(), Some("w"));
    }

    #[test]
    fn works_with_any_prefix_or_default_namespace() {
        let xml = format!("<document xmlns=\"{W}\"><body><p><r><t>hi</t></r></p></body></document>");
        let doc = XmlDocument::parse(xml.as_bytes()).unwrap();
        assert!(doc.root.is(W, "document"));
        assert_eq!(doc.prefix_for(W).as_deref(), Some(""));

        let xml = format!("<ns0:document xmlns:ns0=\"{W}\"><ns0:body/></ns0:document>");
        let doc = XmlDocument::parse(xml.as_bytes()).unwrap();
        assert!(doc.root.child(W, "body").is_some());
        assert_eq!(doc.prefix_for(W).as_deref(), Some("ns0"));
    }

    #[test]
    fn modified_elements_are_reserialized_and_others_untouched() {
        let mut doc = XmlDocument::parse(SAMPLE.as_bytes()).unwrap();
        let body = doc.root.child_mut(W, "body").unwrap();
        let p = body.elements_mut().find(|e| e.is(W, "p")).unwrap();
        p.set_attr("w:rsidR", Some(W), "00CD");
        let t = p.child_mut(W, "r").unwrap().child_mut(W, "t").unwrap();
        t.set_text("x < y & z");

        let out = String::from_utf8(doc.to_bytes()).unwrap();
        assert!(
            out.contains("<w:p w:rsidR=\"00CD\"><w:r><w:t xml:space=\"preserve\">x &lt; y &amp; z</w:t></w:r></w:p>"),
            "{out}"
        );
        // Everything else is unchanged.
        assert!(out.contains("<w:p/><w:p ></w:p><w:p w:x = 'single \"quoted\"' />"));
        assert!(out.contains("mc:Ignorable=\"w14\">"));
        assert!(out.ends_with("</w:document>\n<!-- trailing -->"));
    }

    #[test]
    fn new_elements_and_attribute_changes() {
        let mut el = Element::new("w", "jc", Some(W));
        el.set_attr("w:val", Some(W), "center");
        let mut out = String::new();
        el.write_to(&mut out);
        assert_eq!(out, "<w:jc w:val=\"center\"/>");

        el.set_attr("w:val", Some(W), "a\"b<c");
        assert!(el.remove_attr(Some(W), "missing") == false);
        let mut out = String::new();
        el.write_to(&mut out);
        assert_eq!(out, "<w:jc w:val=\"a&quot;b&lt;c\"/>");
        assert_eq!(el.attr(Some(W), "val").as_deref(), Some("a\"b<c"));

        let mut parent = Element::new("w", "p", Some(W));
        parent.children.push(Node::Element(el));
        assert!(parent.child(W, "jc").is_some());
        let mut out = String::new();
        parent.write_to(&mut out);
        assert!(out.starts_with("<w:p><w:jc"));
        assert!(out.ends_with("</w:p>"));
    }

    #[test]
    fn rejects_unsafe_or_malformed_input() {
        let cases: &[(&[u8], &str)] = &[
            (b"<!DOCTYPE x [<!ENTITY a \"aaaa\">]><x>&a;</x>", "DOCTYPE"),
            (b"<a><b></a>", ""),
            (b"<a>", "unclosed"),
            (b"", "no root"),
            (b"<a/><b/>", "more than one root"),
            (b"text<a/>", "text outside"),
            (b"<a>\xff</a>", "UTF-8"),
            (b"\xFF\xFE<\x00a\x00/\x00>\x00", "UTF-16"),
            (b"<a x=\"1\" x=\"2\"/>", ""),
        ];
        for (input, needle) in cases {
            let err = XmlDocument::parse(input).unwrap_err();
            assert!(err.0.contains(needle), "{:?}: {err}", String::from_utf8_lossy(input));
        }
    }

    #[test]
    fn escape_text_escapes_markup_only() {
        assert_eq!(escape_text("a & b < c > d \"e\" 'f'"), "a &amp; b &lt; c &gt; d \"e\" 'f'");
    }
}
