//! Character and paragraph formatting changes.
//!
//! A change only touches the properties it names; everything else in the
//! run's `w:rPr` or the paragraph's `w:pPr` (including properties the editor
//! doesn't know, such as language or kerning) is kept. Properties are
//! inserted at their position in the OOXML schema sequence.
//!
//! In JSON, a field that is absent leaves the property unchanged; `null`
//! removes it (so the style's value applies); a value sets it.

use serde::{Deserialize, Deserializer};

use crate::error::{DocxError, Result};
use crate::model::Alignment;
use crate::props::{PPR_ORDER, RPR_ORDER, WNames, ensure_child, ensure_ppr, remove_child, sort_schema};
use crate::xml::{Element, Node};

/// Highlight colours Word supports (`w:highlight` values).
pub const HIGHLIGHTS: [&str; 16] = [
    "yellow",
    "green",
    "cyan",
    "magenta",
    "blue",
    "red",
    "darkBlue",
    "darkCyan",
    "darkGreen",
    "darkMagenta",
    "darkRed",
    "darkYellow",
    "darkGray",
    "lightGray",
    "black",
    "white",
];

/// Absent → `None` (unchanged); `null` → `Some(None)` (remove); value →
/// `Some(Some(v))` (set).
fn nullable<'de, D, T>(d: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// A change to character formatting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFormat {
    #[serde(default)]
    pub bold: Option<bool>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub underline: Option<bool>,
    #[serde(default)]
    pub strike: Option<bool>,
    /// Font family name.
    #[serde(default, deserialize_with = "nullable")]
    pub font: Option<Option<String>>,
    /// Size in half-points (24 = 12 pt).
    #[serde(default, deserialize_with = "nullable")]
    pub size: Option<Option<u32>>,
    /// Hex RGB without `#`, e.g. `C00000`.
    #[serde(default, deserialize_with = "nullable")]
    pub color: Option<Option<String>>,
    /// One of [`HIGHLIGHTS`].
    #[serde(default, deserialize_with = "nullable")]
    pub highlight: Option<Option<String>>,
}

impl RunFormat {
    pub fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(DocxError::InvalidEdit(msg.to_string()));
        if let Some(Some(font)) = &self.font
            && (font.trim().is_empty() || font.chars().count() > 31 || font.chars().any(|c| c.is_control() || c == '"'))
        {
            return invalid("font names must be 1 to 31 characters");
        }
        if let Some(Some(size)) = self.size
            && !(2..=3276).contains(&size)
        {
            return invalid("font sizes must be between 1 and 1638 points");
        }
        if let Some(Some(color)) = &self.color
            && !is_hex_color(color)
        {
            return invalid("colours must be six hexadecimal digits");
        }
        if let Some(Some(h)) = &self.highlight
            && !HIGHLIGHTS.contains(&h.as_str())
        {
            return invalid("unsupported highlight colour");
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn set_toggle(rpr: &mut Element, w: &WNames, local: &str, on: bool) {
    let el = ensure_child(rpr, w, RPR_ORDER, local);
    if on {
        el.remove_attr(Some(&w.ns), "val");
    } else {
        w.set_attr(el, "val", "0");
    }
}

fn set_val(parent: &mut Element, w: &WNames, order: &[&str], local: &str, value: &str) {
    let el = ensure_child(parent, w, order, local);
    w.set_attr(el, "val", value);
}

/// Applies a formatting change to a run's properties (`None` = the run has
/// none yet). Returns the new properties, or `None` when empty.
pub fn apply_run_format(rpr: Option<Element>, w: &WNames, f: &RunFormat) -> Option<Element> {
    let mut rpr = rpr.unwrap_or_else(|| w.element("rPr"));
    if let Some(on) = f.bold {
        set_toggle(&mut rpr, w, "b", on);
        remove_child(&mut rpr, w, "bCs");
    }
    if let Some(on) = f.italic {
        set_toggle(&mut rpr, w, "i", on);
        remove_child(&mut rpr, w, "iCs");
    }
    if let Some(on) = f.underline {
        set_val(&mut rpr, w, RPR_ORDER, "u", if on { "single" } else { "none" });
    }
    if let Some(on) = f.strike {
        remove_child(&mut rpr, w, "dstrike");
        if on {
            set_toggle(&mut rpr, w, "strike", true);
        } else {
            remove_child(&mut rpr, w, "strike");
        }
    }
    match &f.font {
        Some(Some(name)) => {
            let fonts = ensure_child(&mut rpr, w, RPR_ORDER, "rFonts");
            for theme in ["asciiTheme", "hAnsiTheme", "csTheme"] {
                fonts.remove_attr(Some(&w.ns), theme);
            }
            for attr in ["ascii", "hAnsi", "cs"] {
                w.set_attr(fonts, attr, name);
            }
        }
        Some(None) => {
            if let Some(fonts) = rpr.child_mut(&w.ns, "rFonts") {
                for attr in ["ascii", "hAnsi", "cs", "asciiTheme", "hAnsiTheme", "csTheme"] {
                    fonts.remove_attr(Some(&w.ns), attr);
                }
                if fonts.attributes().is_empty() {
                    remove_child(&mut rpr, w, "rFonts");
                }
            }
        }
        None => {}
    }
    match f.size {
        Some(Some(size)) => {
            let value = size.to_string();
            set_val(&mut rpr, w, RPR_ORDER, "sz", &value);
            set_val(&mut rpr, w, RPR_ORDER, "szCs", &value);
        }
        Some(None) => {
            remove_child(&mut rpr, w, "sz");
            remove_child(&mut rpr, w, "szCs");
        }
        None => {}
    }
    match &f.color {
        Some(Some(color)) => {
            let el = ensure_child(&mut rpr, w, RPR_ORDER, "color");
            for theme in ["themeColor", "themeTint", "themeShade"] {
                el.remove_attr(Some(&w.ns), theme);
            }
            w.set_attr(el, "val", &color.to_ascii_uppercase());
        }
        Some(None) => {
            remove_child(&mut rpr, w, "color");
        }
        None => {}
    }
    match &f.highlight {
        Some(Some(h)) => {
            set_val(&mut rpr, w, RPR_ORDER, "highlight", h);
        }
        Some(None) => {
            remove_child(&mut rpr, w, "highlight");
        }
        None => {}
    }
    // A changed run is rewritten anyway; make sure its properties are valid
    // even if the original listed them out of order.
    sort_schema(&mut rpr, w, RPR_ORDER);
    let empty = rpr.children.iter().all(|n| matches!(n, Node::Text(t) if t.trim().is_empty()));
    (!empty).then_some(rpr)
}

/// Indentation in twips. `first_line` is negative for a hanging indent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndentChange {
    #[serde(default)]
    pub left: Option<i32>,
    #[serde(default)]
    pub right: Option<i32>,
    #[serde(default)]
    pub first_line: Option<i32>,
}

/// Spacing: `before`/`after` in twips; `line` in 240ths of a line (240 =
/// single, 360 = 1.5, 480 = double).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpacingChange {
    #[serde(default)]
    pub before: Option<u32>,
    #[serde(default)]
    pub after: Option<u32>,
    #[serde(default)]
    pub line: Option<u32>,
}

/// List types the editor can apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Bullet,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListChange {
    pub kind: ListKind,
    #[serde(default)]
    pub level: u8,
}

/// A change to paragraph formatting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParagraphFormat {
    #[serde(default, deserialize_with = "nullable")]
    pub style_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub alignment: Option<Option<Alignment>>,
    /// Given values are set; omitted ones are kept. `null` removes all.
    #[serde(default, deserialize_with = "nullable")]
    pub indent: Option<Option<IndentChange>>,
    #[serde(default, deserialize_with = "nullable")]
    pub spacing: Option<Option<SpacingChange>>,
    /// `null` removes list formatting.
    #[serde(default, deserialize_with = "nullable")]
    pub list: Option<Option<ListChange>>,
}

impl ParagraphFormat {
    pub fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(DocxError::InvalidEdit(msg.to_string()));
        if let Some(Some(ind)) = &self.indent {
            for v in [ind.left, ind.right, ind.first_line].into_iter().flatten() {
                if !(-31680..=31680).contains(&v) {
                    return invalid("indents must be within 22 inches");
                }
            }
        }
        if let Some(Some(sp)) = &self.spacing {
            for v in [sp.before, sp.after].into_iter().flatten() {
                if v > 31680 {
                    return invalid("paragraph spacing must be at most 1584 points");
                }
            }
            if let Some(line) = sp.line
                && !(12..=31680).contains(&line)
            {
                return invalid("line spacing is out of range");
            }
        }
        if let Some(Some(list)) = &self.list
            && list.level > 8
        {
            return invalid("list levels go from 0 to 8");
        }
        Ok(())
    }
}

/// Applies a paragraph format to `p`. `num_id` is the numbering instance to
/// use when the format sets a list (resolved by the caller).
pub fn apply_paragraph_format(p: &mut Element, w: &WNames, f: &ParagraphFormat, num_id: Option<&str>) {
    let ppr = ensure_ppr(p, w);
    match &f.style_id {
        Some(Some(style)) => {
            set_val(ppr, w, PPR_ORDER, "pStyle", style);
        }
        Some(None) => {
            remove_child(ppr, w, "pStyle");
        }
        None => {}
    }
    match &f.alignment {
        Some(Some(a)) => {
            set_val(ppr, w, PPR_ORDER, "jc", a.to_ooxml());
        }
        Some(None) => {
            remove_child(ppr, w, "jc");
        }
        None => {}
    }
    match &f.indent {
        Some(Some(ind)) => {
            let el = ensure_child(ppr, w, PPR_ORDER, "ind");
            if let Some(left) = ind.left {
                el.remove_attr(Some(&w.ns), "start");
                w.set_attr(el, "left", &left.to_string());
            }
            if let Some(right) = ind.right {
                el.remove_attr(Some(&w.ns), "end");
                w.set_attr(el, "right", &right.to_string());
            }
            if let Some(first) = ind.first_line {
                el.remove_attr(Some(&w.ns), "firstLine");
                el.remove_attr(Some(&w.ns), "hanging");
                if first >= 0 {
                    w.set_attr(el, "firstLine", &first.to_string());
                } else {
                    w.set_attr(el, "hanging", &(-first).to_string());
                }
            }
        }
        Some(None) => {
            remove_child(ppr, w, "ind");
        }
        None => {}
    }
    match &f.spacing {
        Some(Some(sp)) => {
            let el = ensure_child(ppr, w, PPR_ORDER, "spacing");
            if let Some(before) = sp.before {
                el.remove_attr(Some(&w.ns), "beforeAutospacing");
                w.set_attr(el, "before", &before.to_string());
            }
            if let Some(after) = sp.after {
                el.remove_attr(Some(&w.ns), "afterAutospacing");
                w.set_attr(el, "after", &after.to_string());
            }
            if let Some(line) = sp.line {
                w.set_attr(el, "line", &line.to_string());
                w.set_attr(el, "lineRule", "auto");
            }
        }
        Some(None) => {
            remove_child(ppr, w, "spacing");
        }
        None => {}
    }
    match (&f.list, num_id) {
        (Some(Some(list)), Some(num_id)) => {
            let num_pr = ensure_child(ppr, w, PPR_ORDER, "numPr");
            set_val(num_pr, w, &["ilvl", "numId"], "ilvl", &list.level.to_string());
            set_val(num_pr, w, &["ilvl", "numId"], "numId", num_id);
        }
        (Some(None), _) => {
            remove_child(ppr, w, "numPr");
        }
        _ => {}
    }
}

/// Vertical alignment of a cell's content (`w:vAlign`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlign {
    Top,
    Center,
    Bottom,
}

impl VerticalAlign {
    pub fn from_ooxml(value: &str) -> Option<Self> {
        match value {
            "top" => Some(Self::Top),
            "center" => Some(Self::Center),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    pub fn to_ooxml(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Center => "center",
            Self::Bottom => "bottom",
        }
    }
}

/// A change to a table cell's properties.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellFormat {
    #[serde(default, deserialize_with = "nullable")]
    pub vertical_align: Option<Option<VerticalAlign>>,
    /// Background colour, hex RGB without `#`.
    #[serde(default, deserialize_with = "nullable")]
    pub shading: Option<Option<String>>,
}

impl CellFormat {
    pub fn validate(&self) -> Result<()> {
        if let Some(Some(color)) = &self.shading
            && !is_hex_color(color)
        {
            return Err(DocxError::InvalidEdit("colours must be six hexadecimal digits".into()));
        }
        Ok(())
    }
}

fn is_hex_color(color: &str) -> bool {
    color.len() == 6 && color.chars().all(|c| c.is_ascii_hexdigit())
}

/// Paper size in twips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSize {
    pub width: u32,
    pub height: u32,
}

/// Page margins in twips. Omitted sides are unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Margins {
    #[serde(default)]
    pub top: Option<u32>,
    #[serde(default)]
    pub right: Option<u32>,
    #[serde(default)]
    pub bottom: Option<u32>,
    #[serde(default)]
    pub left: Option<u32>,
}

/// A change to the page layout of the document's sections.
///
/// * `size`: the paper size; its orientation follows `landscape` (or the
///   section's current orientation when `landscape` is absent).
/// * `landscape`: turns pages sideways (width and height are swapped as
///   needed).
/// * `page_number_start`: the number of the first page (`null` continues
///   from the previous section, Word's default).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionFormat {
    #[serde(default)]
    pub size: Option<PageSize>,
    #[serde(default)]
    pub landscape: Option<bool>,
    #[serde(default)]
    pub margins: Option<Margins>,
    #[serde(default, deserialize_with = "nullable")]
    pub page_number_start: Option<Option<u32>>,
}

/// Smallest and largest page dimension Word accepts (0.25 in to 22 in).
pub const PAGE_DIMENSION_RANGE: std::ops::RangeInclusive<u32> = 360..=31680;

impl SectionFormat {
    pub fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(DocxError::InvalidEdit(msg.to_string()));
        if let Some(size) = self.size
            && !(PAGE_DIMENSION_RANGE.contains(&size.width) && PAGE_DIMENSION_RANGE.contains(&size.height))
        {
            return invalid("page sizes must be between 0.25 and 22 inches");
        }
        if let Some(m) = self.margins {
            for v in [m.top, m.right, m.bottom, m.left].into_iter().flatten() {
                if v > 31680 {
                    return invalid("margins must be at most 22 inches");
                }
            }
        }
        if let Some(Some(start)) = self.page_number_start
            && start > 32767
        {
            return invalid("page numbers can start at most at 32767");
        }
        Ok(())
    }
}

/// Applies a page layout change to one section's properties (`w:sectPr`),
/// keeping everything else in them.
pub fn apply_section_format(sect: &mut Element, w: &WNames, f: &SectionFormat) -> Result<()> {
    use crate::props::SECTPR_ORDER;
    let ns = w.ns.as_str();
    let number = |el: Option<&Element>, name: &str| {
        el.and_then(|e| e.attr(Some(ns), name)).and_then(|v| v.parse::<f64>().ok()).map(|v| v as i64)
    };

    if f.size.is_some() || f.landscape.is_some() {
        let current = sect.child(ns, "pgSz");
        let mut width = number(current, "w").unwrap_or(11906).clamp(1, 31680) as u32;
        let mut height = number(current, "h").unwrap_or(16838).clamp(1, 31680) as u32;
        let was_landscape =
            current.and_then(|e| e.attr(Some(ns), "orient")).as_deref() == Some("landscape") || width > height;
        if let Some(size) = f.size {
            (width, height) = (size.width, size.height);
        }
        let landscape = f.landscape.unwrap_or(was_landscape);
        let (short, long) = (width.min(height), width.max(height));
        (width, height) = if landscape { (long, short) } else { (short, long) };
        let pg = ensure_child(sect, w, SECTPR_ORDER, "pgSz");
        w.set_attr(pg, "w", &width.to_string());
        w.set_attr(pg, "h", &height.to_string());
        if landscape {
            w.set_attr(pg, "orient", "landscape");
        } else {
            pg.remove_attr(Some(ns), "orient");
        }
    }

    if let Some(m) = f.margins {
        let created = sect.child(ns, "pgMar").is_none();
        let mar = ensure_child(sect, w, SECTPR_ORDER, "pgMar");
        if created {
            // Every page margin attribute is required by the schema.
            for (name, value) in [
                ("top", "1440"),
                ("right", "1440"),
                ("bottom", "1440"),
                ("left", "1440"),
                ("header", "708"),
                ("footer", "708"),
                ("gutter", "0"),
            ] {
                w.set_attr(mar, name, value);
            }
        }
        for (name, value) in [("top", m.top), ("right", m.right), ("bottom", m.bottom), ("left", m.left)] {
            if let Some(v) = value {
                w.set_attr(mar, name, &v.to_string());
            }
        }
    }

    if f.size.is_some() || f.landscape.is_some() || f.margins.is_some() {
        let pg = sect.child(ns, "pgSz");
        let mar = sect.child(ns, "pgMar");
        let width = number(pg, "w").unwrap_or(11906);
        let height = number(pg, "h").unwrap_or(16838);
        let side = |name: &str| number(mar, name).unwrap_or(1440).abs();
        if side("left") + side("right") + 360 > width || side("top") + side("bottom") + 360 > height {
            return Err(DocxError::InvalidEdit("the margins leave no room for text on the page".into()));
        }
    }

    match f.page_number_start {
        Some(Some(start)) => {
            let el = ensure_child(sect, w, SECTPR_ORDER, "pgNumType");
            w.set_attr(el, "start", &start.to_string());
        }
        Some(None) => {
            if let Some(el) = sect.child_mut(ns, "pgNumType") {
                el.remove_attr(Some(ns), "start");
                if el.attributes().is_empty() && el.children.is_empty() {
                    remove_child(sect, w, "pgNumType");
                }
            }
        }
        None => {}
    }
    Ok(())
}
