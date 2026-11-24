// Keyboard access to the interface, beyond the document itself.
//
// * Toolbars follow the ARIA toolbar pattern: each toolbar is one Tab
//   stop, and the arrow keys (and Home, End) move between its controls.
//   The control last used stays the toolbar's Tab stop.
// * F6 and Shift+F6 move the focus between the regions of the window
//   (toolbars, the find bar, notices, the document), and Alt+F10 goes to
//   the formatting toolbar, as in other editors.

/**
 * The index the focus moves to in a toolbar of `count` controls, from
 * `current`, for `key`; null when the key doesn't move it.
 */
export function toolbarIndex(count, current, key) {
  if (!count) return null;
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return (current + 1) % count;
    case "ArrowLeft":
    case "ArrowUp":
      return (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/** Whether an element is shown (it and its ancestors aren't hidden). */
function shown(el) {
  return Boolean(el) && !el.closest("[hidden]") && el.getClientRects().length > 0;
}

/** Focusable controls of a toolbar, in order. */
function controlsOf(toolbar) {
  return [...toolbar.querySelectorAll("button, select, input:not([type=file])")].filter((c) => !c.disabled && shown(c));
}

/**
 * Makes `toolbar` a single Tab stop navigated with the arrow keys.
 * Returns `sync()`, which keeps one enabled control tabbable after
 * controls were enabled, disabled, shown or hidden.
 */
export function rovingToolbar(toolbar) {
  let stop = null;

  function sync() {
    const controls = controlsOf(toolbar);
    if (!controls.includes(stop)) stop = controls[0] || null;
    for (const c of toolbar.querySelectorAll("button, select, input:not([type=file])")) c.tabIndex = c === stop ? 0 : -1;
  }

  toolbar.addEventListener("focusin", (event) => {
    const control = event.target.closest("button, select, input");
    if (control && control !== stop && controlsOf(toolbar).includes(control)) {
      stop = control;
      sync();
    }
  });

  toolbar.addEventListener("keydown", (event) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    // Up and Down change a menu's value; only the other keys move.
    const target = event.target;
    if (target.tagName === "SELECT" && (event.key === "ArrowUp" || event.key === "ArrowDown")) return;
    if (target.tagName === "INPUT" && target.type !== "color" && target.type !== "checkbox") return;
    const controls = controlsOf(toolbar);
    const next = toolbarIndex(controls.length, controls.indexOf(target), event.key);
    if (next === null) return;
    event.preventDefault();
    controls[next].focus();
  });

  // Controls are enabled and disabled as the document and the cursor change.
  new MutationObserver(sync).observe(toolbar, { subtree: true, attributes: true, attributeFilter: ["disabled", "hidden"] });
  sync();
  return sync;
}

/**
 * F6 / Shift+F6 cycle the focus through `regions()` (elements, in order;
 * hidden ones are skipped), and Alt+F10 calls `toToolbar()`.
 * @param {() => HTMLElement[]} regions
 * @param {() => void} toToolbar
 */
export function enableRegionKeys(regions, toToolbar) {
  document.addEventListener("keydown", (event) => {
    if (event.key === "F10" && event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      toToolbar();
      return;
    }
    if (event.key !== "F6" || event.altKey || event.ctrlKey || event.metaKey) return;
    const all = regions().filter(shown);
    if (!all.length) return;
    event.preventDefault();
    const current = all.findIndex((r) => r.contains(document.activeElement));
    const next = current < 0 ? 0 : (current + (event.shiftKey ? -1 : 1) + all.length) % all.length;
    focusRegion(all[next]);
  });
}

/** Moves the focus into a region: its tabbable control, or the region itself. */
export function focusRegion(region) {
  if (region.isContentEditable || region.tabIndex >= 0 && region.matches("[contenteditable], input, select, button")) {
    region.focus();
    return;
  }
  const target =
    region.querySelector('[tabindex="0"]:not([disabled])') ||
    [...region.querySelectorAll("button, select, input:not([type=file]), [contenteditable=true]")].find((c) => !c.disabled && shown(c));
  if (target) target.focus();
}
