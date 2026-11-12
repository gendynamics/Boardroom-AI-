"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { createPage } = require("./fake-dom.js");

const source = fs.readFileSync(path.join(__dirname, "../public/neon-pal.js"), "utf8");

async function createDockPage() {
  let document;
  const loaded = await createPage(`globalThis.capture(document);${source}`, {}, {
    globalThis: { capture(value) { document = value; } },
  });
  const visit = (id) => document.dispatchEvent({ type: "boardroom:page", detail: { id } });
  const press = (key) => document.dispatchEvent({ type: "keydown", key });
  return { ...loaded, visit, press };
}

test("the launcher opens one shared chat in the dock on any page other than the Copilot page", async () => {
  const { $, visit } = await createDockPage();
  visit("business-overview");
  assert.equal($("#pal-launcher").hidden, false);
  assert.equal($("#pal-dock").hidden, true);

  $("#pal-launcher").listeners.click();
  assert.equal($("#pal-dock").hidden, false);
  assert.equal($("#pal-launcher").hidden, true);
  assert.deepEqual($("#pal-dock-body").children, [$("#pal-chat")]);
  assert.equal($("#ai-question").focused, true);
  assert.equal($("#pal-launcher").attributes["aria-expanded"], "true");
});

test("the dock stays open across pages and closes with its button or Escape", async () => {
  const { $, visit, press } = await createDockPage();
  visit("activity");
  $("#pal-launcher").listeners.click();
  visit("invoicing");
  assert.equal($("#pal-dock").hidden, false);

  $("#pal-close").listeners.click();
  assert.equal($("#pal-dock").hidden, true);
  assert.equal($("#pal-launcher").hidden, false);

  $("#pal-launcher").listeners.click();
  press("Escape");
  assert.equal($("#pal-dock").hidden, true);
});

test("on the Copilot page the chat returns to the page and the launcher is hidden", async () => {
  const { $, visit } = await createDockPage();
  visit("wallet");
  $("#pal-launcher").listeners.click();
  visit("boardroom-ai");
  assert.equal($("#pal-dock").hidden, true);
  assert.equal($("#pal-launcher").hidden, true);
  assert.deepEqual($("#pal-home").children, [$("#pal-chat")]);

  $("#pal-launcher").listeners.click();
  assert.equal($("#pal-dock").hidden, true);
});

test("a disabled chat input (AI off) is not focused when the dock opens", async () => {
  const { $, visit } = await createDockPage();
  $("#ai-question").disabled = true;
  visit("home");
  $("#pal-launcher").listeners.click();
  assert.equal($("#pal-dock").hidden, false);
  assert.notEqual($("#ai-question").focused, true);
});
