// Choosing files to open and saving files on the user's computer.
//
// Where the browser supports the File System Access API (Chromium-based
// browsers), opening a file keeps a handle so that "Save" writes back to the
// same file. Elsewhere, files are opened with a file input and saved as
// downloads. The page only ever touches files the user picks.

export const DOCX_ACCEPT = {
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document": [".docx"],
  "application/vnd.ms-word.document.macroEnabled.12": [".docm"],
  "application/vnd.openxmlformats-officedocument.wordprocessingml.template": [".dotx"],
  "application/vnd.ms-word.template.macroEnabled.12": [".dotm"],
};

const PICKER_TYPES = [{ description: "Word documents", accept: DOCX_ACCEPT }];
const PICKER_ID = "docx-editor";

/** Whether the browser can open and save files in place. */
export function supportsFileSystemAccess(win = globalThis) {
  return typeof win.showOpenFilePicker === "function" && typeof win.showSaveFilePicker === "function";
}

function isAbort(err) {
  return err && err.name === "AbortError";
}

/**
 * Asks the user for a Word file.
 * @returns {Promise<{file: File, handle: FileSystemFileHandle|null}|null>} null if cancelled.
 */
export async function pickFile(win = globalThis, doc = globalThis.document) {
  if (supportsFileSystemAccess(win)) {
    try {
      const [handle] = await win.showOpenFilePicker({ id: PICKER_ID, types: PICKER_TYPES, multiple: false });
      return { file: await handle.getFile(), handle };
    } catch (err) {
      if (isAbort(err)) return null;
      throw err;
    }
  }
  return pickWithInput(doc);
}

function pickWithInput(doc) {
  return new Promise((resolve) => {
    const input = doc.createElement("input");
    input.type = "file";
    input.accept = Object.values(DOCX_ACCEPT).flat().concat(Object.keys(DOCX_ACCEPT)).join(",");
    input.addEventListener("change", () => {
      const file = input.files && input.files[0];
      resolve(file ? { file, handle: null } : null);
    });
    input.addEventListener("cancel", () => resolve(null));
    input.click();
  });
}

/**
 * Asks where to save. Returns a handle, or null if cancelled.
 * Only available with the File System Access API.
 */
export async function pickSaveLocation(suggestedName, win = globalThis) {
  try {
    return await win.showSaveFilePicker({ id: PICKER_ID, suggestedName, types: PICKER_TYPES });
  } catch (err) {
    if (isAbort(err)) return null;
    throw err;
  }
}

/** Writes a blob to a file handle (replacing its contents). */
export async function writeToHandle(handle, blob) {
  const writable = await handle.createWritable();
  try {
    await writable.write(blob);
  } catch (err) {
    await writable.abort?.();
    throw err;
  }
  await writable.close();
}

/** A readable explanation of why writing `name` failed. */
export function saveErrorMessage(err, name) {
  switch (err && err.name) {
    case "NotAllowedError":
    case "SecurityError":
      return `Permission to write to “${name}” was not given. Use Save as… to choose where to save it.`;
    case "NoModificationAllowedError":
    case "InvalidStateError":
    case "InvalidModificationError":
      return `“${name}” couldn't be written: it may be open in another program (such as Word) or read-only. Close it there, or use Save as… to save a copy.`;
    case "QuotaExceededError":
      return `“${name}” couldn't be written because the disk is full.`;
    case "NotFoundError":
      return `“${name}” no longer exists where it was opened from. Use Save as… to choose where to save it.`;
    default:
      return `“${name}” couldn't be saved: ${(err && err.message) || err}`;
  }
}

/** Saves a blob through the browser's download mechanism. */
export function downloadBlob(blob, name, doc = globalThis.document, url = globalThis.URL) {
  const href = url.createObjectURL(blob);
  const link = doc.createElement("a");
  link.href = href;
  link.download = name;
  link.rel = "noopener";
  doc.body.appendChild(link);
  link.click();
  link.remove();
  // Give the browser time to start the download before releasing the blob.
  setTimeout(() => url.revokeObjectURL(href), 10000);
}
