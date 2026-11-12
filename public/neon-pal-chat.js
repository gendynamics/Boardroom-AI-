"use strict";
// neon-pal-chat.js - loads boardroom_dataset.jsonl and fills the four board seats.
(() => {
  const $ = (s) => document.querySelector(s);
  const SLOTS = {
    Strategist: "#np-take-strategist",
    Marketer: "#np-take-marketer",
    CFO: "#np-take-cfo",
    "Growth Lead": "#np-take-growth",
  };
  let dataset = [];

  function setStatus(msg) {
    const el = $("#np-status");
    if (el) el.textContent = msg;
  }

  async function loadDataset() {
    const res = await fetch("boardroom_dataset.jsonl");
    if (!res.ok) throw new Error("dataset fetch failed: " + res.status);
    const text = await res.text();
    dataset = text.trim().split("\n").map((line) => {
      const row = JSON.parse(line);
      return { brief: row.messages[1].content, takes: JSON.parse(row.messages[2].content) };
    });
    setStatus("Neon Pal online - " + dataset.length + " plays loaded");
  }

  // Keyword-overlap lookup. Swap this for a model API call once you fine-tune.
  function findBest(query) {
    const words = new Set(query.toLowerCase().match(/[a-z0-9]+/g) || []);
    let best = null;
    let bestScore = 0;
    for (const row of dataset) {
      const bw = new Set(row.brief.toLowerCase().match(/[a-z0-9]+/g) || []);
      let score = 0;
      bw.forEach((w) => { if (w.length > 2 && words.has(w)) score += 1; });
      if (score > bestScore) { bestScore = score; best = row; }
    }
    return best || dataset[Math.floor(Math.random() * dataset.length)];
  }

  function bubble(role, text) {
    const log = $("#neon-pal-log");
    if (!log) return;
    const div = document.createElement("div");
    div.className = "np-msg np-" + role;
    div.textContent = text;
    log.appendChild(div);
    log.scrollTop = log.scrollHeight;
  }

  function typeInto(el, text) {
    clearInterval(el._timer);
    el.textContent = "";
    let i = 0;
    el._timer = setInterval(() => {
      if (i >= text.length) { clearInterval(el._timer); return; }
      el.textContent += text[i];
      i += 1;
    }, 10);
  }

  async function ask(question) {
    if (!dataset.length) { setStatus("Dataset not loaded"); return; }
    bubble("user", question);
    setStatus("The board is deliberating...");
    document.dispatchEvent(new CustomEvent("neon-pal:thinking"));
    await new Promise((r) => setTimeout(r, 900));
    const match = findBest(question);
    Object.keys(SLOTS).forEach((seat, i) => {
      const el = $(SLOTS[seat]);
      if (el) setTimeout(() => typeInto(el, match.takes[seat] || ""), i * 400);
    });
    bubble("bot", "Neon Pal: " + match.takes.recommendation);
    setStatus("Board recommendation ready");
    document.dispatchEvent(new CustomEvent("neon-pal:answered", { detail: match }));
  }

  document.addEventListener("DOMContentLoaded", async () => {
    const form = $("#neon-pal-form");
    const input = $("#neon-pal-input");
    if (form && input) {
      form.addEventListener("submit", (e) => {
        e.preventDefault();
        const q = input.value.trim();
        if (!q) return;
        input.value = "";
        ask(q);
      });
    }
    try {
      await loadDataset();
    } catch (err) {
      setStatus("Dataset not found in public/");
      console.error(err);
    }
  });

  window.NeonPal = { ask };
})();
