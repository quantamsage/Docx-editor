import { test } from "node:test";
import assert from "node:assert/strict";

import {
  connectionLabel,
  countWords,
  createConnectionMonitor,
  formatPagePosition,
  formatWordCount,
} from "../../web/js/status.js";

test("countWords counts words like a word processor", () => {
  const cases = [
    ["", 0],
    ["   \n\t ", 0],
    [null, 0],
    ["Hello", 1],
    ["Hello, world!", 2],
    ["It's a well-known fact.", 4],
    ["Line one\nLine two", 4],
    ["Prices: 3.50 and 100", 4],
    ["Ünïcödé wörds ça va", 4],
    ["— – - ...", 0],
  ];
  for (const [text, expected] of cases) {
    assert.equal(countWords(text), expected, JSON.stringify(text));
  }
});

test("formatWordCount pluralizes and groups digits", () => {
  assert.equal(formatWordCount(0), "0 words");
  assert.equal(formatWordCount(1), "1 word");
  assert.equal(formatWordCount(12345), "12,345 words");
});

test("formatPagePosition clamps to a valid range", () => {
  assert.equal(formatPagePosition(1, 1), "Page 1 of 1");
  assert.equal(formatPagePosition(3, 5), "Page 3 of 5");
  assert.equal(formatPagePosition(9, 5), "Page 5 of 5");
  assert.equal(formatPagePosition(0, 0), "Page 1 of 1");
});

test("connectionLabel describes each state", () => {
  assert.equal(connectionLabel({ state: "connecting" }), "Connecting to local server…");
  assert.equal(connectionLabel({ state: "connected", version: "0.1.0" }), "Local server v0.1.0");
  assert.equal(connectionLabel({ state: "connected" }), "Local server connected");
  assert.equal(connectionLabel({ state: "offline", message: "Cannot reach it" }), "Cannot reach it");
  assert.equal(connectionLabel({ state: "offline" }), "Local server unavailable");
});

/** Manual timers so polling can be stepped deterministically. */
function fakeTimers() {
  const pending = new Map();
  let nextId = 1;
  return {
    pending,
    setTimer(fn, ms) {
      const id = nextId++;
      pending.set(id, { fn, ms });
      return id;
    },
    clearTimer(id) {
      pending.delete(id);
    },
    async fireNext() {
      const [id, timer] = pending.entries().next().value;
      pending.delete(id);
      await timer.fn();
      return timer.ms;
    },
  };
}

test("connection monitor reports connect, disconnect and recovery", async () => {
  const timers = fakeTimers();
  const outcomes = [
    { version: "0.1.0" },
    new Error("Cannot reach the local editor server."),
    new Error("Cannot reach the local editor server."),
    { version: "0.1.0" },
  ];
  const changes = [];
  const monitor = createConnectionMonitor({
    check: async () => {
      const next = outcomes.shift();
      if (next instanceof Error) throw next;
      return next;
    },
    onChange: (status) => changes.push(status),
    intervalMs: 15000,
    retryMs: 3000,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });

  await monitor.checkNow();
  assert.deepEqual(changes, [{ state: "connected", version: "0.1.0" }]);
  assert.equal([...timers.pending.values()][0].ms, 15000, "polls slowly while connected");

  await timers.fireNext();
  assert.equal(changes.at(-1).state, "offline");
  assert.equal([...timers.pending.values()][0].ms, 3000, "retries quickly while offline");

  await timers.fireNext();
  assert.equal(changes.length, 2, "repeated failures are not reported twice");

  await timers.fireNext();
  assert.deepEqual(changes.at(-1), { state: "connected", version: "0.1.0" });
  assert.equal(timers.pending.size, 1, "exactly one poll is scheduled");

  monitor.stop();
  assert.equal(timers.pending.size, 0, "stop cancels the next poll");
});

test("checkNow replaces a scheduled poll instead of adding one", async () => {
  const timers = fakeTimers();
  const monitor = createConnectionMonitor({
    check: async () => ({ version: "1" }),
    onChange: () => {},
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  await monitor.checkNow();
  await monitor.checkNow();
  await monitor.checkNow();
  assert.equal(timers.pending.size, 1);
});
