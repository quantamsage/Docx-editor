// Builds a long document for performance tests through the editor's own
// API: a report of `sections` sections, each a Heading 1 and twelve
// paragraphs of mixed formatting (about 55 words each), with a filled 6 × 4
// table after every second section. 250 sections are about 330 pages in
// Word (3,000 paragraphs, 125 tables, 165,000 words).

const WORDS = "the quarterly results show steady growth across all regions while operating costs remained within the planned budget and the team delivered every milestone on schedule with improved customer satisfaction scores".split(" ");

/** The edits that turn a blank document into the report. */
export function largeDocumentEdits(sections = 250) {
  let seed = 7;
  const rand = (n) => ((seed = (seed * 1103515245 + 12345) % 2147483648), seed % n);
  const sentence = (n) => Array.from({ length: n }, () => WORDS[rand(WORDS.length)]).join(" ");
  const content = [];
  const headings = [];
  const tablesAt = [];
  let block = 0;
  for (let s = 0; s < sections; s++) {
    if (block) content.push({ type: "break" });
    content.push({ type: "text", text: `Section ${s + 1}: ${sentence(4)}` });
    headings.push(block++);
    for (let p = 0; p < 12; p++) {
      content.push({ type: "break" });
      content.push({ type: "text", text: `${sentence(18)} ` });
      content.push({ type: "text", text: sentence(3), format: { bold: true } });
      content.push({ type: "text", text: ` ${sentence(20)} ` });
      content.push({ type: "text", text: sentence(4), format: { italic: true, color: "1F4E79" } });
      content.push({ type: "text", text: ` ${sentence(8)}.` });
      block++;
    }
    if (s % 2 === 1) tablesAt.push(block);
  }
  const edits = [{ op: "splice", first: 0, last: 0, content }];
  for (const h of headings) edits.push({ op: "format_paragraphs", first: h, last: h, format: { style_id: "Heading1" } });
  for (const at of [...tablesAt].reverse()) edits.push({ op: "insert_table", at, rows: 6, columns: 4 });
  tablesAt.forEach((at, k) => {
    const table = at + k;
    for (let r = 0; r < 6; r++) {
      for (let c = 0; c < 4; c++) {
        const text = r === 0 ? `Column ${c + 1}` : String(rand(9000) + 100);
        edits.push({ op: "splice", container: { cell: { table, row: r, cell: c } }, first: 0, last: 0, content: [{ type: "text", text }] });
      }
    }
  });
  return edits;
}

/** Creates the document on the server at `base` and returns its .docx bytes. */
export async function buildLargeDocument(base, sections = 250) {
  const headers = { Origin: base, "content-type": "application/json" };
  const ok = async (response) => {
    if (!response.ok) throw new Error(`${response.status} ${await response.text()}`);
    return response;
  };
  let doc = await (await ok(await fetch(`${base}/api/documents/new`, { method: "POST", headers }))).json();
  const edits = largeDocumentEdits(sections);
  for (let i = 0; i < edits.length; i += 1000) {
    const body = JSON.stringify({ base_revision: doc.revision, edits: edits.slice(i, i + 1000) });
    doc = await (await ok(await fetch(`${base}/api/documents/${doc.id}/edits`, { method: "POST", headers, body }))).json();
  }
  const bytes = Buffer.from(await (await ok(await fetch(`${base}/api/documents/${doc.id}/download`))).arrayBuffer());
  await fetch(`${base}/api/documents/${doc.id}`, { method: "DELETE", headers });
  return bytes;
}
