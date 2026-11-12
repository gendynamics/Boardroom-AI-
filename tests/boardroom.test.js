"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { createPage, flush, response } = require("./fake-dom");

const source = fs.readFileSync(path.join(__dirname, "../public/boardroom.js"), "utf8");

const BRIEF = {
  headline: "Turn first-time buyers into repeat buyers",
  summary: "You captured $90.00 USD from 2 orders.",
  experiment: {
    title: "Returning-buyer bundle", audience: "Your 2 recent buyers", offer: "A $40 bundle link",
    duration: "7 days", guardrail: "Stop after 5 sales", successMeasure: "Paid bundle links",
  },
  perspectives: [
    { role: "Strategist", take: "Keep it small." },
    { role: "Marketer", take: "Send a personal note." },
    { role: "CFO", take: "Cap the discount." },
    { role: "Growth Lead", take: "Measure paid links." },
  ],
  verdict: "Run it for a week.",
  copilotRequest: "Create a $40 payment link for an Everyday Tote bundle.",
  generatedAt: "2026-11-08T10:00:00Z",
  model: "xAI Grok · grok-4.7",
  basis: { days: 30, paidOrders: 2 },
};

test("without an AI key no brief can be written and no sample brief is shown", async () => {
  const { $ } = await createPage(source, {
    "/api/boardroom/brief": () => response(200, { brief: null, aiConfigured: false }),
  });
  assert.equal($("#brief-generate").disabled, true);
  assert.equal($("#experiment-send").disabled, true);
  assert.equal($("#brief-content").hidden, true);
  assert.match($("#brief-summary").textContent, /XAI_API_KEY/);
});

test("a saved brief renders, and its first step reaches Copilot only when sent", async () => {
  const { $, events, window } = await createPage(source, {
    "/api/boardroom/brief": () => response(200, { brief: BRIEF, aiConfigured: true }),
  });
  assert.equal($("#brief-headline").textContent, BRIEF.headline);
  assert.equal($("#take-growth").textContent, "Measure paid links.");
  assert.equal($("#experiment-request").textContent, BRIEF.copilotRequest);
  assert.equal($("#experiment-status").textContent, "AWAITING REVIEW");
  assert.equal(events.length, 0);

  await $("#experiment-send").listeners.click();
  const ask = events.find((event) => event.type === "boardroom:ask");
  assert.equal(ask.detail.prompt, BRIEF.copilotRequest);
  assert.equal(window.location.hash, "#boardroom-ai");
  assert.equal($("#experiment-status").textContent, "SENT TO COPILOT");
});

test("a failed brief is reported without a scripted fallback", async () => {
  const { $ } = await createPage(source, {
    "/api/boardroom/brief": () => response(200, { brief: null, aiConfigured: true }),
    "POST /api/boardroom/brief": () => response(502, { error: "The AI returned an unusable brief. Try again." }),
  });
  assert.equal($("#brief-generate").disabled, false);
  await $("#brief-generate").listeners.click();
  await flush();
  assert.match($("#brief-status").textContent, /unusable brief.*No scripted brief/);
  assert.equal($("#brief-content").hidden, true);
  assert.equal($("#brief-generate").disabled, false);
});
