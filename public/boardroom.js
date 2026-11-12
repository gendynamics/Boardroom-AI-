"use strict";

// The Boardroom brief: the server asks the AI to read the dashboard figures (real Sandbox
// activity) and write one recommendation. The first step goes to Boardroom Copilot, which still
// needs the merchant's approval for anything it creates.
(() => {
  const $ = (selector) => document.querySelector(selector);
  const generateButton = $("#brief-generate");
  const sendButton = $("#experiment-send");
  const status = $("#brief-status");
  const TAKES = {
    Strategist: "#take-strategist",
    Marketer: "#take-marketer",
    CFO: "#take-cfo",
    "Growth Lead": "#take-growth",
  };
  let brief = null;
  let aiConfigured = false;
  let busy = false;

  async function readJson(response) {
    let data = {};
    try { data = await response.json(); } catch { data = {}; }
    if (!response.ok) throw new Error(data.error || `Request failed (HTTP ${response.status}).`);
    return data;
  }

  function when(iso) {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? "" : date.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  }

  function render() {
    const has = Boolean(brief);
    $("#brief-content").hidden = !has;
    $("#experiment-details").hidden = !has;
    sendButton.disabled = !has || busy;
    generateButton.disabled = !aiConfigured || busy;
    generateButton.textContent = has ? "Write a new brief" : "Write a brief from my data";
    if (!has) {
      $("#brief-headline").textContent = "No brief yet";
      $("#brief-summary").textContent = aiConfigured
        ? "Ask the Boardroom for a brief. The AI reads the figures on your dashboard and writes one recommendation."
        : "The Boardroom needs an AI key on the server. Add XAI_API_KEY or OPENAI_API_KEY to .env and restart.";
      $("#brief-basis").lastChild.textContent = " From your Sandbox data";
      $("#experiment-status").textContent = "NO BRIEF YET";
      $("#experiment-copy").textContent = "Write a brief first. Its experiment and first step will appear here for your review.";
      return;
    }
    $("#brief-headline").textContent = brief.headline;
    $("#brief-summary").textContent = brief.summary;
    const basis = brief.basis || {};
    $("#brief-basis").lastChild.textContent = ` ${basis.paidOrders ?? 0} paid order${basis.paidOrders === 1 ? "" : "s"} · ${when(brief.generatedAt)}`;
    const experiment = brief.experiment;
    $("#brief-experiment-title").textContent = experiment.title;
    $("#brief-experiment-offer").textContent = `${experiment.offer} · ${experiment.audience}`;
    $("#brief-experiment-duration").textContent = experiment.duration.toUpperCase().slice(0, 16);
    for (const perspective of brief.perspectives) {
      const target = TAKES[perspective.role];
      if (target) $(target).textContent = perspective.take;
    }
    $("#brief-verdict").textContent = brief.verdict;
    $("#experiment-title").textContent = experiment.title;
    $("#experiment-audience").textContent = experiment.audience;
    $("#experiment-offer").textContent = experiment.offer;
    $("#experiment-duration").textContent = experiment.duration;
    $("#experiment-guardrail").textContent = experiment.guardrail;
    $("#experiment-measure").textContent = experiment.successMeasure;
    $("#experiment-request").textContent = brief.copilotRequest;
    $("#experiment-copy").textContent = `Written by ${brief.model || "the AI"} from your Sandbox figures. Review it before acting; AI advice can be wrong.`;
    if ($("#experiment-status").textContent !== "SENT TO COPILOT") $("#experiment-status").textContent = "AWAITING REVIEW";
  }

  async function load() {
    try {
      const data = await readJson(await fetch("/api/boardroom/brief"));
      brief = data.brief || null;
      aiConfigured = Boolean(data.aiConfigured);
      render();
    } catch (error) {
      status.textContent = `${error.message} Open the app through its server.`;
    }
  }

  generateButton.addEventListener("click", async () => {
    if (busy) return;
    busy = true;
    render();
    status.textContent = "The Boardroom is reading your Sandbox figures…";
    try {
      const data = await readJson(await fetch("/api/boardroom/brief", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{}",
      }));
      brief = data.brief;
      $("#experiment-status").textContent = "AWAITING REVIEW";
      status.textContent = "New brief written from your latest figures.";
      if (typeof CustomEvent === "function" && typeof document.dispatchEvent === "function") {
        document.dispatchEvent(new CustomEvent("boardroom:changed"));
      }
    } catch (error) {
      status.textContent = `${error.message} No scripted brief was substituted.`;
    } finally {
      busy = false;
      render();
    }
  });

  sendButton.addEventListener("click", () => {
    if (!brief) return;
    $("#experiment-status").textContent = "SENT TO COPILOT";
    if (typeof CustomEvent === "function" && typeof document.dispatchEvent === "function") {
      document.dispatchEvent(new CustomEvent("boardroom:ask", { detail: { prompt: brief.copilotRequest } }));
    }
    window.location.hash = "#boardroom-ai";
  });

  load();
})();
