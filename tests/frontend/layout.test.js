import { test } from "node:test";
import assert from "node:assert/strict";

import {
  cmToTwips,
  marginPresetId,
  marginsOf,
  paperId,
  pictureSize,
  resizedPicture,
  sectionFormat,
  textWidthTwips,
  twipsToCm,
} from "../../web/js/layout.js";

const a4 = { page_width: 11906, page_height: 16838, margin_top: 1440, margin_right: 1440, margin_bottom: 1440, margin_left: 1440 };

test("units and presets", () => {
  assert.equal(twipsToCm(1440), 2.54);
  assert.equal(cmToTwips(2.54), 1440);
  assert.equal(cmToTwips("1.27"), 720);
  assert.equal(paperId(a4), "a4");
  assert.equal(paperId({ page_width: 15840, page_height: 12240, landscape: true }), "letter", "orientation doesn't matter");
  assert.equal(paperId({ page_width: 10000, page_height: 14000 }), "custom");
  assert.equal(paperId(null), "a4", "Word's default");
  assert.equal(marginPresetId(a4), "normal");
  assert.equal(marginPresetId({ ...a4, margin_left: 720, margin_right: 720, margin_top: 720, margin_bottom: 720 }), "narrow");
  assert.equal(marginPresetId({ ...a4, margin_left: 1000 }), "custom");
  assert.deepEqual(marginsOf({ margin_top: -1440 }), { top: 1440, right: 1440, bottom: 1440, left: 1440 }, "negative top margins are fixed margins");
  assert.equal(textWidthTwips(a4), 9026);
});

test("page setup builds the section format", () => {
  const margins = { top: 2.54, right: 1.27, bottom: 2.54, left: 1.27 };
  assert.deepEqual(sectionFormat({ paper: "letter", landscape: true, margins, section: a4 }), {
    format: { landscape: true, margins: { top: 1440, right: 720, bottom: 1440, left: 720 }, size: { width: 12240, height: 15840 } },
  });
  const custom = sectionFormat({ paper: "custom", landscape: false, margins, section: { page_width: 10000, page_height: 14000 } });
  assert.equal(custom.format.size, undefined, "a custom paper size is kept");
  assert.match(sectionFormat({ paper: "a4", landscape: false, margins: { ...margins, left: 12, right: 10 }, section: a4 }).error, /no room/);
  assert.match(sectionFormat({ paper: "a4", landscape: false, margins: { ...margins, top: "x" }, section: a4 }).error, /between/);
  assert.match(sectionFormat({ paper: "a4", landscape: false, margins: { ...margins, top: -1 }, section: a4 }).error, /between/);
  // Landscape A4 leaves 29.7 cm across: 12 + 10 cm side margins fit.
  assert.ok(sectionFormat({ paper: "a4", landscape: true, margins: { ...margins, left: 12, right: 10 }, section: a4 }).format);
});

test("pictures fit the text width and keep their proportions", () => {
  assert.deepEqual(pictureSize(100, 50, a4), { width_emu: 952500, height_emu: 476250 });
  const wide = pictureSize(2000, 1000, a4);
  assert.equal(wide.width_emu, 9026 * 635);
  assert.equal(wide.height_emu, Math.round((9026 * 635) / 2));
  assert.deepEqual(resizedPicture(200, 100, 50), { width: 250, height: 125 });
  assert.deepEqual(resizedPicture(200, 100, -500), { width: 8, height: 4 }, "never smaller than 8 px");
  assert.deepEqual(resizedPicture(200, 100, 500, 300), { width: 300, height: 150 }, "never wider than the page");
});
