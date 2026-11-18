// Entry point: wires the interface to the local server.

import { getHealth } from "./api.js";
import { connectionLabel, countWords, createConnectionMonitor, formatPagePosition, formatWordCount } from "./status.js";

function $(id) {
  return document.getElementById(id);
}

function renderConnection(status) {
  const indicator = $("status-connection");
  indicator.dataset.state = status.state;
  $("status-connection-text").textContent = connectionLabel(status);
}

function renderDocumentStats() {
  const pages = document.querySelectorAll("#workspace .page");
  // Until a document is open the page shows a welcome message, not content.
  const content = document.querySelector("#workspace .page .document-content");
  $("status-page").textContent = formatPagePosition(1, pages.length);
  $("status-words").textContent = formatWordCount(content ? countWords(content.textContent) : 0);
}

function init() {
  renderDocumentStats();
  const monitor = createConnectionMonitor({ check: getHealth, onChange: renderConnection });
  monitor.checkNow();
  // Re-check straight away when the window regains focus (e.g. after the
  // server was restarted), instead of waiting for the next poll.
  window.addEventListener("focus", () => monitor.checkNow());
}

init();
