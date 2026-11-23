// Containers: where a paragraph lives. The engine addresses paragraphs by
// their index among the blocks of a container: the document body, a table
// cell, or a header or footer part (or a cell of a table in one). No DOM
// access, so it is unit-tested in Node.
//
// On the page, every editable paragraph carries `data-block` (its index in
// its container) and, outside the body, `data-container` with the
// container's key: "" for the body, otherwise a small JSON text.

/**
 * @typedef {{part?: string|null, cell?: {table: number, row: number, cell: number}|null}} Container
 */

/** The key of a container ("" for the body). */
export function containerKey(container) {
  const part = container && container.part ? container.part : null;
  const cell = container && container.cell ? container.cell : null;
  if (!part && !cell) return "";
  const out = {};
  if (part) out.part = part;
  if (cell) out.cell = [cell.table, cell.row, cell.cell];
  return JSON.stringify(out);
}

/** The container a key stands for ({} for the body). */
export function parseContainerKey(key) {
  if (!key) return {};
  const raw = JSON.parse(key);
  const out = {};
  if (raw.part) out.part = raw.part;
  if (raw.cell) out.cell = { table: raw.cell[0], row: raw.cell[1], cell: raw.cell[2] };
  return out;
}

/** The key of a cell of a table in the container with key `parentKey`. */
export function cellKey(parentKey, table, row, cell) {
  return containerKey({ ...parseContainerKey(parentKey), cell: { table, row, cell } });
}

/** The key of the container a cell key's table is in. */
export function parentKey(key) {
  const { part } = parseContainerKey(key);
  return containerKey({ part });
}

/** Whether the key is (or is inside) a header or footer. */
export function isHeaderFooterKey(key) {
  return Boolean(parseContainerKey(key).part);
}

/** The blocks of the container with key `key`, or null if it doesn't exist. */
export function containerBlocks(model, key) {
  const { part, cell } = parseContainerKey(key);
  let blocks = model.blocks || [];
  if (part) {
    const hf = [model.header, model.footer].find((h) => h && h.part === part);
    if (!hf) return null;
    blocks = hf.blocks || [];
  }
  if (cell) {
    const table = blocks[cell.table];
    const target = table && table.type === "table" ? table.rows?.[cell.row]?.cells?.[cell.cell] : null;
    return target ? target.blocks || [] : null;
  }
  return blocks;
}

/**
 * The edit fields that address a container: `{}` for the body (the
 * default), otherwise `{container: {...}}`.
 */
export function containerField(key) {
  return key ? { container: parseContainerKey(key) } : {};
}
