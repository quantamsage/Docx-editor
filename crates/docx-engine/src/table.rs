//! Table editing: new tables, inserting and deleting rows and columns,
//! borders and cell properties.
//!
//! A table (`w:tbl`) has a grid of columns (`w:tblGrid`) and rows of cells;
//! a cell covers `w:gridSpan` grid columns and may continue a vertical merge
//! from the cell above (`w:vMerge`). Row and column edits keep these
//! consistent: spans shrink or grow, merges are kept intact, the total table
//! width stays the same and fixed cell widths (`w:tcW` in twips) follow the
//! grid. Everything else in the table (properties, cell content, unknown
//! elements) is kept.

use crate::error::{DocxError, Result};
use crate::format::CellFormat;
use crate::props::{TBLBORDERS_ORDER, TBLPR_ORDER, TCPR_ORDER, WNames, ensure_child, remove_child};
use crate::xml::{Element, Node};

/// Most columns Word allows in a table.
pub const MAX_COLUMNS: usize = 63;
/// Most rows accepted when inserting a table.
pub const MAX_NEW_ROWS: usize = 500;

fn invalid(msg: impl Into<String>) -> DocxError {
    DocxError::InvalidEdit(msg.into())
}

fn el_mut(node: &mut Node) -> &mut Element {
    match node {
        Node::Element(e) => e,
        _ => unreachable!("index points at an element"),
    }
}

/// A new table with `rows` × `columns` empty cells spread over `width`
/// twips, with single borders. `style` is a table style to apply (e.g.
/// Table Grid) when the document defines one.
pub fn build_table(w: &WNames, rows: usize, columns: usize, width: u32, style: Option<&str>) -> Result<Element> {
    if !(1..=MAX_NEW_ROWS).contains(&rows) || !(1..=MAX_COLUMNS).contains(&columns) {
        return Err(invalid(format!("tables can have 1 to {MAX_NEW_ROWS} rows and 1 to {MAX_COLUMNS} columns")));
    }
    let widths = split_width(width.max(columns as u32 * 100), columns);
    let mut tbl = w.element("tbl");
    let mut tbl_pr = w.element("tblPr");
    if let Some(style) = style {
        let mut el = w.element("tblStyle");
        w.set_attr(&mut el, "val", style);
        tbl_pr.children.push(Node::Element(el));
    }
    let mut tbl_w = w.element("tblW");
    w.set_attr(&mut tbl_w, "w", "0");
    w.set_attr(&mut tbl_w, "type", "auto");
    tbl_pr.children.push(Node::Element(tbl_w));
    set_borders(&mut tbl_pr, w, true);
    let mut look = w.element("tblLook");
    for (name, value) in [
        ("val", "04A0"),
        ("firstRow", "1"),
        ("lastRow", "0"),
        ("firstColumn", "1"),
        ("lastColumn", "0"),
        ("noHBand", "0"),
        ("noVBand", "1"),
    ] {
        w.set_attr(&mut look, name, value);
    }
    tbl_pr.children.push(Node::Element(look));
    tbl.children.push(Node::Element(tbl_pr));

    let mut grid = w.element("tblGrid");
    for width in &widths {
        let mut col = w.element("gridCol");
        w.set_attr(&mut col, "w", &width.to_string());
        grid.children.push(Node::Element(col));
    }
    tbl.children.push(Node::Element(grid));
    for _ in 0..rows {
        let mut tr = w.element("tr");
        for width in &widths {
            let mut tc = w.element("tc");
            let mut tc_pr = w.element("tcPr");
            let mut tc_w = w.element("tcW");
            w.set_attr(&mut tc_w, "w", &width.to_string());
            w.set_attr(&mut tc_w, "type", "dxa");
            tc_pr.children.push(Node::Element(tc_w));
            tc.children.push(Node::Element(tc_pr));
            tc.children.push(Node::Element(w.element("p")));
            tr.children.push(Node::Element(tc));
        }
        tbl.children.push(Node::Element(tr));
    }
    Ok(tbl)
}

/// Splits `total` into `n` integer parts that add up to it.
fn split_width(total: u32, n: usize) -> Vec<u32> {
    let n32 = n as u32;
    (0..n32).map(|i| total / n32 + u32::from(i < total % n32)).collect()
}

/// Scales widths so that they add up to `total` (largest remainder).
fn scale_widths(widths: &[u32], total: u32) -> Vec<u32> {
    let sum: u64 = widths.iter().map(|&w| u64::from(w.max(1))).sum();
    let mut out: Vec<u32> = widths.iter().map(|&w| (u64::from(w.max(1)) * u64::from(total) / sum) as u32).collect();
    let mut rest = total.saturating_sub(out.iter().sum());
    let mut i = 0;
    while rest > 0 && !out.is_empty() {
        out[i % widths.len()] += 1;
        rest -= 1;
        i += 1;
    }
    out
}

/// A table's grid widths and, per row, each cell's first grid column and
/// span.
struct Layout {
    grid: Vec<u32>,
    rows: Vec<Vec<(usize, usize)>>,
}

fn grid_widths(tbl: &Element, w: &WNames) -> Option<Vec<u32>> {
    let grid = tbl.child(&w.ns, "tblGrid")?;
    Some(
        grid.elements()
            .filter(|c| c.is(&w.ns, "gridCol"))
            .map(|c| c.attr(Some(&w.ns), "w").and_then(|v| v.parse::<f64>().ok()).map_or(0, |v| v.max(0.0) as u32))
            .collect(),
    )
}

fn span_of(tc: &Element, w: &WNames) -> usize {
    tc.child(&w.ns, "tcPr")
        .and_then(|p| p.child(&w.ns, "gridSpan"))
        .and_then(|e| e.attr(Some(&w.ns), "val"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1usize)
        .max(1)
}

fn rows<'t>(tbl: &'t Element, w: &WNames) -> impl Iterator<Item = &'t Element> {
    tbl.elements().filter(|e| e.is(&w.ns, "tr"))
}

fn cells<'t>(tr: &'t Element, w: &WNames) -> impl Iterator<Item = &'t Element> {
    tr.elements().filter(|e| e.is(&w.ns, "tc"))
}

/// Reads the layout of a table whose structure the editor can change
/// safely: a grid, rows and cells directly inside, and no rows shifted
/// with `gridBefore`/`gridAfter`.
fn layout(tbl: &Element, w: &WNames) -> Result<Layout> {
    let unsupported = || {
        invalid(
            "this table's layout can't be changed by the editor (it uses features such as content controls around rows or shifted rows)",
        )
    };
    let grid = grid_widths(tbl, w).filter(|g| !g.is_empty()).ok_or_else(unsupported)?;
    let markers =
        ["bookmarkStart", "bookmarkEnd", "proofErr", "commentRangeStart", "commentRangeEnd", "permStart", "permEnd"];
    let mut rows_layout = Vec::new();
    for child in tbl.elements() {
        if child.is(&w.ns, "tblPr")
            || child.is(&w.ns, "tblGrid")
            || child.namespace.as_deref() == Some(w.ns.as_str()) && markers.contains(&child.local_name())
        {
            continue;
        }
        if !child.is(&w.ns, "tr") {
            return Err(unsupported());
        }
        if let Some(tr_pr) = child.child(&w.ns, "trPr")
            && (tr_pr.child(&w.ns, "gridBefore").is_some() || tr_pr.child(&w.ns, "gridAfter").is_some())
        {
            return Err(unsupported());
        }
        let mut col = 0;
        let mut row = Vec::new();
        for cell in child.elements() {
            if cell.is(&w.ns, "tc") {
                let span = span_of(cell, w);
                row.push((col, span));
                col += span;
            } else if !(cell.is(&w.ns, "trPr") || cell.is(&w.ns, "tblPrEx") || markers.contains(&cell.local_name())) {
                return Err(unsupported());
            }
        }
        rows_layout.push(row);
    }
    Ok(Layout { grid, rows: rows_layout })
}

fn row_mut<'t>(tbl: &'t mut Element, w: &WNames, row: usize) -> Option<&'t mut Element> {
    tbl.elements_mut().filter(|e| e.is(&w.ns, "tr")).nth(row)
}

fn cell_mut<'r>(tr: &'r mut Element, w: &WNames, cell: usize) -> Option<&'r mut Element> {
    tr.elements_mut().filter(|e| e.is(&w.ns, "tc")).nth(cell)
}

fn no_cell(row: usize, cell: usize) -> DocxError {
    invalid(format!("the table has no cell {cell} in row {row}"))
}

/// `"restart"`, `"continue"` or `None` for a cell's vertical merge.
fn vmerge(tc: &Element, w: &WNames) -> Option<String> {
    tc.child(&w.ns, "tcPr")
        .and_then(|p| p.child(&w.ns, "vMerge"))
        .map(|e| e.attr(Some(&w.ns), "val").unwrap_or_else(|| "continue".into()))
}

fn set_vmerge(tc: &mut Element, w: &WNames, value: Option<&str>) {
    let tc_pr = ensure_tc_pr(tc, w);
    match value {
        Some(v) => {
            let el = ensure_child(tc_pr, w, TCPR_ORDER, "vMerge");
            if v == "continue" {
                el.remove_attr(Some(&w.ns), "val");
            } else {
                w.set_attr(el, "val", v);
            }
        }
        None => {
            remove_child(tc_pr, w, "vMerge");
        }
    }
}

fn ensure_tc_pr<'a>(tc: &'a mut Element, w: &WNames) -> &'a mut Element {
    let index = match tc.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(&w.ns, "tcPr"))) {
        Some(i) => i,
        None => {
            tc.children.insert(0, Node::Element(w.element("tcPr")));
            0
        }
    };
    el_mut(&mut tc.children[index])
}

/// An empty cell modelled on `template`: the same cell properties (except
/// spans and merges) and a paragraph with the properties of the template's
/// first paragraph.
fn empty_cell_like(template: &Element, w: &WNames) -> Element {
    let mut tc = w.element("tc");
    if let Some(tc_pr) = template.child(&w.ns, "tcPr") {
        let mut tc_pr = tc_pr.clone();
        for local in ["gridSpan", "hMerge", "vMerge", "cellIns", "cellDel", "cellMerge", "tcPrChange"] {
            remove_child(&mut tc_pr, w, local);
        }
        tc.children.push(Node::Element(tc_pr));
    }
    let mut p = w.element("p");
    if let Some(ppr) = template.child(&w.ns, "p").and_then(|p| p.child(&w.ns, "pPr")) {
        let mut ppr = ppr.clone();
        remove_child(&mut ppr, w, "pPrChange");
        if !ppr.children.is_empty() {
            p.children.push(Node::Element(ppr));
        }
    }
    tc.children.push(Node::Element(p));
    tc
}

/// Inserts an empty row above (`after = false`) or below row `row`.
pub fn insert_row(tbl: &mut Element, w: &WNames, row: usize, after: bool) -> Result<()> {
    let layout = layout(tbl, w)?;
    let count = layout.rows.len();
    if row >= count {
        return Err(invalid(format!("the table has no row {row}")));
    }
    let template = rows(tbl, w).nth(row).expect("counted").clone();
    // A new cell continues a vertical merge when the cell below it does
    // (it is then inside the merged block).
    let below_row = if after { row + 1 } else { row };
    let below: Vec<(usize, Option<String>)> = rows(tbl, w)
        .nth(below_row)
        .map(|tr| cells(tr, w).zip(&layout.rows[below_row]).map(|(tc, (start, _))| (*start, vmerge(tc, w))).collect())
        .unwrap_or_default();

    let mut tr = w.element("tr");
    for child in template.elements() {
        if child.is(&w.ns, "tblPrEx") {
            tr.children.push(Node::Element(child.clone()));
        } else if child.is(&w.ns, "trPr") {
            let mut tr_pr = child.clone();
            for local in ["tblHeader", "ins", "del", "trPrChange", "cnfStyle"] {
                remove_child(&mut tr_pr, w, local);
            }
            if !tr_pr.children.is_empty() {
                tr.children.push(Node::Element(tr_pr));
            }
        }
    }
    for (tc, (start, span)) in cells(&template, w).zip(&layout.rows[row]) {
        let mut cell = empty_cell_like(tc, w);
        if *span > 1 {
            let tc_pr = ensure_tc_pr(&mut cell, w);
            w.set_attr(ensure_child(tc_pr, w, TCPR_ORDER, "gridSpan"), "val", &span.to_string());
        }
        let continues = below.iter().any(|(s, m)| s == start && m.as_deref() == Some("continue"));
        if continues {
            set_vmerge(&mut cell, w, Some("continue"));
        }
        tr.children.push(Node::Element(cell));
    }

    let index = tbl
        .children
        .iter()
        .enumerate()
        .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(&w.ns, "tr")))
        .nth(row)
        .map(|(i, _)| i)
        .expect("counted");
    tbl.children.insert(if after { index + 1 } else { index }, Node::Element(tr));
    Ok(())
}

/// Deletes row `row`, keeping vertical merges that started in it.
pub fn delete_row(tbl: &mut Element, w: &WNames, row: usize) -> Result<()> {
    let layout = layout(tbl, w)?;
    if row >= layout.rows.len() {
        return Err(invalid(format!("the table has no row {row}")));
    }
    if layout.rows.len() == 1 {
        return Err(invalid("a table needs at least one row; delete the table instead"));
    }
    // A merge that starts in this row restarts in the next one.
    let restarts: Vec<usize> = cells(rows(tbl, w).nth(row).expect("counted"), w)
        .zip(&layout.rows[row])
        .filter(|(tc, _)| vmerge(tc, w).as_deref() == Some("restart"))
        .map(|(_, (start, _))| *start)
        .collect();
    if let Some(next_layout) = layout.rows.get(row + 1) {
        let next = row_mut(tbl, w, row + 1).expect("counted");
        for (index, (start, _)) in next_layout.iter().enumerate() {
            if restarts.contains(start) {
                let tc = cell_mut(next, w, index).expect("counted");
                if vmerge(tc, w).as_deref() == Some("continue") {
                    set_vmerge(tc, w, Some("restart"));
                }
            }
        }
    }
    let index = tbl
        .children
        .iter()
        .enumerate()
        .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(&w.ns, "tr")))
        .nth(row)
        .map(|(i, _)| i)
        .expect("counted");
    tbl.children.remove(index);
    Ok(())
}

/// The grid columns covered by cell `cell` of row `row`.
fn cell_columns(layout: &Layout, row: usize, cell: usize) -> Result<(usize, usize)> {
    layout.rows.get(row).and_then(|r| r.get(cell)).copied().ok_or_else(|| no_cell(row, cell))
}

/// Inserts a grid column left (`after = false`) or right of cell
/// `cell` of row `row`. The new column is as wide as its neighbour and the
/// table keeps its total width.
pub fn insert_column(tbl: &mut Element, w: &WNames, row: usize, cell: usize, after: bool) -> Result<()> {
    let layout = layout(tbl, w)?;
    let (start, span) = cell_columns(&layout, row, cell)?;
    if layout.grid.len() >= MAX_COLUMNS {
        return Err(invalid(format!("tables can have at most {MAX_COLUMNS} columns")));
    }
    let at = if after { start + span } else { start };
    let reference = if after { start + span - 1 } else { start };

    for (r, row_layout) in layout.rows.iter().enumerate() {
        let tr = row_mut(tbl, w, r).expect("counted");
        // Inside a cell spanning the new column's position: widen it.
        if let Some(index) = row_layout.iter().position(|(s, n)| *s < at && at < s + n) {
            let tc = cell_mut(tr, w, index).expect("counted");
            let span = row_layout[index].1 + 1;
            let tc_pr = ensure_tc_pr(tc, w);
            w.set_attr(ensure_child(tc_pr, w, TCPR_ORDER, "gridSpan"), "val", &span.to_string());
            continue;
        }
        // Otherwise a new cell next to the neighbouring one.
        let before_index = row_layout.iter().position(|(s, _)| *s == at);
        let template_index = before_index.unwrap_or(row_layout.len() - 1);
        let template = cells(tr, w).nth(template_index).expect("counted").clone();
        let new_cell = empty_cell_like(&template, w);
        let position = match before_index {
            Some(i) => child_index_of_cell(tr, w, i),
            None => child_index_of_cell(tr, w, row_layout.len() - 1) + 1,
        };
        tr.children.insert(position, Node::Element(new_cell));
    }

    let mut grid = layout.grid.clone();
    grid.insert(at, layout.grid[reference]);
    let total: u32 = layout.grid.iter().sum();
    set_grid(tbl, w, &scale_widths(&grid, total.max(grid.len() as u32)));
    Ok(())
}

/// Deletes the grid column where cell `cell` of row `row` starts. Cells
/// spanning it get narrower; the others in it are removed. The remaining
/// columns share the freed width.
pub fn delete_column(tbl: &mut Element, w: &WNames, row: usize, cell: usize) -> Result<()> {
    let layout = layout(tbl, w)?;
    let (column, _) = cell_columns(&layout, row, cell)?;
    if layout.grid.len() == 1 {
        return Err(invalid("a table needs at least one column; delete the table instead"));
    }
    for (r, row_layout) in layout.rows.iter().enumerate() {
        let Some(index) = row_layout.iter().position(|(s, n)| *s <= column && column < s + n) else { continue };
        let tr = row_mut(tbl, w, r).expect("counted");
        let span = row_layout[index].1;
        if span > 1 {
            let tc = cell_mut(tr, w, index).expect("counted");
            let tc_pr = ensure_tc_pr(tc, w);
            if span == 2 {
                remove_child(tc_pr, w, "gridSpan");
            } else {
                w.set_attr(ensure_child(tc_pr, w, TCPR_ORDER, "gridSpan"), "val", &(span - 1).to_string());
            }
        } else if row_layout.len() > 1 {
            let position = child_index_of_cell(tr, w, index);
            tr.children.remove(position);
        } else {
            return Err(invalid("deleting this column would leave a row without cells"));
        }
    }
    let mut grid = layout.grid.clone();
    grid.remove(column);
    let total: u32 = layout.grid.iter().sum();
    set_grid(tbl, w, &scale_widths(&grid, total.max(grid.len() as u32)));
    Ok(())
}

fn child_index_of_cell(tr: &Element, w: &WNames, cell: usize) -> usize {
    tr.children
        .iter()
        .enumerate()
        .filter(|(_, n)| matches!(n, Node::Element(e) if e.is(&w.ns, "tc")))
        .nth(cell)
        .map(|(i, _)| i)
        .expect("counted")
}

/// Writes the grid widths and makes every fixed cell width (`w:tcW` in
/// twips) match the columns the cell covers.
fn set_grid(tbl: &mut Element, w: &WNames, widths: &[u32]) {
    if let Some(grid) = tbl.child_mut(&w.ns, "tblGrid") {
        let mut template =
            grid.elements().find(|e| e.is(&w.ns, "gridCol")).cloned().unwrap_or_else(|| w.element("gridCol"));
        template.children.clear();
        let others: Vec<Node> = grid
            .children
            .iter()
            .filter(|n| !matches!(n, Node::Element(e) if e.is(&w.ns, "gridCol")))
            .cloned()
            .collect();
        grid.children = widths
            .iter()
            .map(|width| {
                let mut col = template.clone();
                w.set_attr(&mut col, "w", &width.to_string());
                Node::Element(col)
            })
            .collect();
        // Extensions (e.g. a grid change record) stay after the columns.
        grid.children.extend(others);
    }
    for tr in tbl.elements_mut().filter(|e| e.is(&w.ns, "tr")) {
        let mut column = 0;
        for tc in tr.elements_mut().filter(|e| e.is(&w.ns, "tc")) {
            let span = span_of(tc, w);
            let width: u32 = widths.iter().skip(column).take(span).sum();
            column += span;
            if let Some(tc_w) = tc.child_mut(&w.ns, "tcPr").and_then(|p| p.child_mut(&w.ns, "tcW")) {
                let kind = tc_w.attr(Some(&w.ns), "type");
                if kind.is_none() || kind.as_deref() == Some("dxa") {
                    w.set_attr(tc_w, "w", &width.to_string());
                }
            }
        }
    }
}

/// Sets all six table borders to a single line, or switches them off.
fn set_borders(tbl_pr: &mut Element, w: &WNames, on: bool) {
    let borders = ensure_child(tbl_pr, w, TBLPR_ORDER, "tblBorders");
    let keep: Vec<Node> = borders
        .children
        .iter()
        .filter(|n| match n {
            Node::Element(e) => {
                e.namespace.as_deref() != Some(w.ns.as_str()) || !TBLBORDERS_ORDER.contains(&e.local_name())
            }
            _ => false,
        })
        .cloned()
        .collect();
    borders.children.clear();
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        let mut el = w.element(side);
        if on {
            for (name, value) in [("val", "single"), ("sz", "4"), ("space", "0"), ("color", "auto")] {
                w.set_attr(&mut el, name, value);
            }
        } else {
            w.set_attr(&mut el, "val", "nil");
        }
        borders.children.push(Node::Element(el));
    }
    borders.children.extend(keep);
}

/// Shows or hides the borders of a table.
pub fn set_table_borders(tbl: &mut Element, w: &WNames, on: bool) {
    let index = match tbl.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(&w.ns, "tblPr"))) {
        Some(i) => i,
        None => {
            tbl.children.insert(0, Node::Element(w.element("tblPr")));
            0
        }
    };
    set_borders(el_mut(&mut tbl.children[index]), w, on);
}

/// The cell `cell` of row `row`.
pub fn table_cell_mut<'t>(tbl: &'t mut Element, w: &WNames, row: usize, cell: usize) -> Result<&'t mut Element> {
    row_mut(tbl, w, row).and_then(|tr| cell_mut(tr, w, cell)).ok_or_else(|| no_cell(row, cell))
}

/// Applies cell properties (vertical alignment, shading).
pub fn format_cell(tc: &mut Element, w: &WNames, f: &CellFormat) {
    let tc_pr = ensure_tc_pr(tc, w);
    match f.vertical_align {
        Some(Some(v)) => w.set_attr(ensure_child(tc_pr, w, TCPR_ORDER, "vAlign"), "val", v.to_ooxml()),
        Some(None) => {
            remove_child(tc_pr, w, "vAlign");
        }
        None => {}
    }
    match &f.shading {
        Some(Some(color)) => {
            let shd = ensure_child(tc_pr, w, TCPR_ORDER, "shd");
            for theme in ["themeFill", "themeFillTint", "themeFillShade"] {
                shd.remove_attr(Some(&w.ns), theme);
            }
            w.set_attr(shd, "val", "clear");
            w.set_attr(shd, "color", "auto");
            w.set_attr(shd, "fill", &color.to_ascii_uppercase());
        }
        Some(None) => {
            remove_child(tc_pr, w, "shd");
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_split_and_scale_exactly() {
        assert_eq!(split_width(9026, 3), [3009, 3009, 3008]);
        assert_eq!(scale_widths(&[3000, 3000, 3000, 3000], 9000), [2250, 2250, 2250, 2250]);
        let scaled = scale_widths(&[1000, 2000, 3001], 9026);
        assert_eq!(scaled.iter().sum::<u32>(), 9026);
        assert!(scaled[0] < scaled[1] && scaled[1] < scaled[2]);
    }
}
