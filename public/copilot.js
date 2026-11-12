"use strict";

// Boardroom Copilot (shown to the merchant as "Neon Pal"): chat with an AI agent that reads PayPal Sandbox data and drafts actions.
// The server runs the agent; this page shows its steps and renders each proposed action as a
// review card. Only the merchant's click on Approve asks the server to call PayPal.
(() => {
  const form = document.querySelector("#ai-form");
  const input = document.querySelector("#ai-question");
  const conversation = document.querySelector("#ai-conversation");
  const statusList = document.querySelector("#copilot-status");
  const badge = document.querySelector("#copilot-badge");
  const suggestions = document.querySelector("#copilot-suggestions");
  const submitButton = form.querySelector("button");
  const BOT_NAME = "Neon Pal";
  const MAX_HISTORY = 12;
  const MAX_MESSAGE_CHARS = 2000;
  const STATUS_TEXT = {
    proposed: "Awaiting your approval",
    executing: "Working…",
    done: "Done",
    failed: "Failed",
    dismissed: "Dismissed",
  };
  const history = [];
  let busy = false;

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined && text !== null) element.textContent = String(text);
    return element;
  }

  async function readJson(response) {
    let data = {};
    try {
      data = await response.json();
    } catch {
      data = {};
    }
    if (!response.ok) {
      const error = new Error(data.error || `Request failed (HTTP ${response.status}).`);
      error.data = data;
      throw error;
    }
    return data;
  }

  async function postJson(url, body) {
    return readJson(await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body || {}),
    }));
  }

  function addMessage(author, text, className) {
    const message = node("div", `ai-message ${className}`);
    message.append(node("strong", "", author), node("p", "", text));
    conversation.append(message);
    message.scrollIntoView({ block: "nearest" });
    return message;
  }

  function stepsList(steps) {
    const list = node("ul", "copilot-steps");
    list.setAttribute("aria-label", `What ${BOT_NAME} checked`);
    for (const step of steps) {
      list.append(node("li", step.ok ? "step-ok" : "step-failed", `${step.ok ? "✓" : "✕"} ${step.label}`));
    }
    return list;
  }

  function isPayPalUrl(url) {
    return typeof url === "string" && /^https:\/\/([a-z0-9-]+\.)*paypal\.com\//i.test(url);
  }

  async function sendInvoice(button, note, invoiceId) {
    button.disabled = true;
    note.textContent = "Sending the Sandbox invoice…";
    try {
      await postJson(`/api/sandbox/invoices/${encodeURIComponent(invoiceId)}/send`, {});
      button.hidden = true;
      note.textContent = `PayPal accepted the send request for ${invoiceId}. Email delivery is not confirmed; check Sandbox for its status.`;
      announceChange();
    } catch (error) {
      note.textContent = `${error.message} Check Sandbox before retrying.`;
    }
  }

  function toCents(amount) {
    const match = /^(\d+)\.(\d{2})$/.exec(String(amount));
    return match ? Number(match[1]) * 100 + Number(match[2]) : 0;
  }

  function formatUsd(cents) {
    const dollars = String(Math.floor(cents / 100)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    return `$${dollars}.${String(cents % 100).padStart(2, "0")}`;
  }

  function reminderCount(count) {
    return `${count} reminder${count === 1 ? "" : "s"}`;
  }

  const OUTCOME_TEXT = { reminded: "Reminder sent", skipped: "Skipped", failed: "Failed" };

  // A collections run lists one reminder per overdue invoice. While it awaits approval the
  // merchant can untick invoices; the total and the approve button follow the selection.
  function collectionsList(action, onChange) {
    const editable = action.status === "proposed";
    const outcomes = new Map();
    for (const entry of (action.result && action.result.results) || []) outcomes.set(entry.invoice_id, entry);
    const list = node("ul", "action-items collection-items");
    const boxes = [];
    for (const item of action.collections.items) {
      const cents = toCents(item.amount_due);
      const row = node("li", item.selected ? "" : "collection-off");
      const label = node("label", "collection-pick");
      const box = node("input");
      box.type = "checkbox";
      box.checked = Boolean(item.selected);
      box.disabled = !editable;
      box.value = item.invoice_id;
      box.setAttribute("aria-label", `Remind ${item.recipient} about ${item.invoice_id}`);
      box.addEventListener("change", () => {
        row.className = box.checked ? "" : "collection-off";
        onChange();
      });
      boxes.push({ box, cents });
      const days = `${item.days_overdue} day${item.days_overdue === 1 ? "" : "s"} overdue`;
      const text = node("span");
      text.append(
        node("strong", "", item.recipient),
        node("small", "", `${item.invoice_id} · ${days} · ${item.reason}`),
        node("small", "collection-subject", item.subject),
        node("small", "collection-note", item.note),
      );
      const outcome = outcomes.get(item.invoice_id);
      if (outcome) {
        const detail = outcome.message ? `: ${outcome.message}` : "";
        text.append(node("small", `collection-outcome outcome-${outcome.outcome}`, `${OUTCOME_TEXT[outcome.outcome] || outcome.outcome}${detail}`));
      }
      label.append(box, text);
      row.append(label, node("span", "action-amount", formatUsd(cents)));
      list.append(row);
    }
    const selection = () => {
      const picked = boxes.filter((entry) => entry.box.checked);
      return {
        ids: picked.map((entry) => entry.box.value),
        cents: picked.reduce((sum, entry) => sum + entry.cents, 0),
      };
    };
    return { list, selection };
  }

  function resultParts(action) {
    const result = action.result || {};
    if (action.kind === "collections_run") {
      const unsent = (result.requested || 0) - (result.reminded || 0);
      const rest = unsent > 0 ? ` ${unsent} not sent; see each invoice above.` : "";
      return [node("p", "action-note", `PayPal accepted ${reminderCount(result.reminded || 0)}.${rest} Email delivery is not confirmed; check Sandbox.`)];
    }
    if (action.kind === "payment_link" && isPayPalUrl(result.approvalUrl)) {
      const link = node("a", "button button-primary action-link", "Open PayPal Sandbox checkout ↗");
      link.href = result.approvalUrl;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      const note = node("p", "action-note", `Order ${result.orderId} created. Pay with a Sandbox buyer account; this app captures the payment when PayPal sends the buyer back. Then ask Neon Pal whether it was paid.`);
      return [link, note];
    }
    if (action.kind === "invoice" && result.invoiceId) {
      const send = node("button", "button button-primary", "Send invoice");
      send.type = "button";
      const note = node("p", "action-note", `Draft ${result.invoiceId} created in PayPal Sandbox. Review it there if you like, then send it.`);
      send.addEventListener("click", () => sendInvoice(send, note, result.invoiceId));
      return [send, note];
    }
    if (action.kind === "refund") {
      return [node("p", "action-note", `PayPal refund ${result.refundId || "(no ID returned)"}: ${result.status || "status unknown"} · ${result.amount || action.total}.`)];
    }
    return [node("p", "action-note", "Done. Check PayPal Sandbox for details.")];
  }

  function drawAction(card, action, message) {
    card.className = `action-card action-${action.status}`;
    card.setAttribute("aria-label", action.title);
    const header = node("div", "action-card-header");
    header.append(
      node("strong", "", action.title),
      node("span", `action-status status-${action.status}`, STATUS_TEXT[action.status] || action.status),
    );
    const parts = [header];

    if (Array.isArray(action.fields) && action.fields.length) {
      const fields = node("dl", "action-fields");
      for (const field of action.fields) fields.append(node("dt", "", field.label), node("dd", "", field.value));
      parts.push(fields);
    }
    if (Array.isArray(action.items) && action.items.length) {
      const items = node("ul", "action-items");
      for (const item of action.items) {
        const row = node("li");
        const label = node("span");
        label.append(node("strong", "", item.name), node("small", "", item.detail));
        row.append(label, node("span", "action-amount", item.amount));
        items.append(row);
      }
      parts.push(items);
    }
    const isRun = Boolean(action.collections && Array.isArray(action.collections.items));
    const totalValue = node("strong", "", action.total);
    const approve = node("button", "button button-primary", action.approveLabel || "Approve");
    let run = null;
    if (isRun) {
      const follow = () => {
        const picked = run.selection();
        totalValue.textContent = `${formatUsd(picked.cents)} USD`;
        approve.textContent = `Send ${reminderCount(picked.ids.length)}`;
        approve.disabled = picked.ids.length === 0;
      };
      run = collectionsList(action, follow);
      parts.push(run.list);
      follow();
    }
    const total = node("div", "action-total");
    const totalLabel = isRun ? "Amount due, selected" : action.kind === "refund" ? "Refund" : "Total";
    total.append(node("span", "", totalLabel), totalValue);
    parts.push(total);

    const footer = node("div", "action-footer");
    if (action.status === "proposed") {
      const dismiss = node("button", "button button-secondary", "Dismiss");
      approve.type = "button";
      dismiss.type = "button";
      approve.addEventListener("click", () => decide(
        card, action, "approve", [approve, dismiss],
        run ? { invoice_ids: run.selection().ids } : {},
      ));
      dismiss.addEventListener("click", () => decide(card, action, "dismiss", [approve, dismiss]));
      footer.append(approve, dismiss, node("p", "action-note", "Nothing has been sent to PayPal yet."));
    } else if (action.status === "done") {
      footer.append(...resultParts(action));
    } else if (action.status === "failed") {
      footer.append(node("p", "action-note action-error", action.error || "PayPal did not confirm this action. Check Sandbox before asking again."));
    } else if (action.status === "dismissed") {
      footer.append(node("p", "action-note", "Dismissed. Nothing was sent to PayPal."));
    }
    if (message) footer.append(node("p", "action-note action-error", message));
    parts.push(footer);
    card.replaceChildren(...parts);
  }

  function announceChange() {
    if (typeof CustomEvent === "function" && typeof document.dispatchEvent === "function") {
      document.dispatchEvent(new CustomEvent("boardroom:changed"));
    }
  }

  async function decide(card, action, verb, buttons, body) {
    for (const button of buttons) button.disabled = true;
    try {
      const data = await postJson(`/api/copilot/actions/${encodeURIComponent(action.id)}/${verb}`, body || {});
      drawAction(card, data.action);
      announceChange();
    } catch (error) {
      const latest = error.data && error.data.action;
      if (latest) drawAction(card, latest, latest.status === "failed" ? "" : error.message);
      else drawAction(card, action, `${error.message} If you approved, check Sandbox before trying again.`);
    }
  }

  function actionCard(action) {
    const card = node("article", "action-card");
    drawAction(card, action);
    return card;
  }

  async function ask(question) {
    const text = String(question || "").trim().slice(0, MAX_MESSAGE_CHARS);
    if (!text || busy) return;
    busy = true;
    submitButton.disabled = true;
    input.value = "";
    addMessage("You", text, "ai-user");
    history.push({ role: "user", content: text });
    const reply = addMessage(BOT_NAME, "Checking PayPal Sandbox…", "ai-advisor ai-thinking");
    try {
      const data = await postJson("/api/copilot/chat", { messages: history.slice(-MAX_HISTORY) });
      const parts = [node("strong", "", BOT_NAME)];
      if (Array.isArray(data.steps) && data.steps.length) parts.push(stepsList(data.steps));
      parts.push(node("p", "", data.reply));
      for (const action of data.actions || []) parts.push(actionCard(action));
      reply.className = "ai-message ai-advisor";
      reply.replaceChildren(...parts);
      history.push({ role: "assistant", content: String(data.reply).slice(0, MAX_MESSAGE_CHARS) });
    } catch (error) {
      history.pop();
      const parts = [node("strong", "", `${BOT_NAME} · error`)];
      if (error.data && Array.isArray(error.data.steps) && error.data.steps.length) parts.push(stepsList(error.data.steps));
      parts.push(node("p", "", `${error.message} No scripted answer was substituted.`));
      reply.className = "ai-message ai-advisor ai-error";
      reply.replaceChildren(...parts);
    } finally {
      busy = false;
      submitButton.disabled = input.disabled;
      reply.scrollIntoView({ block: "nearest" });
      input.focus();
    }
  }

  function statusItem(state, text) {
    return node("li", `status-${state}`, text);
  }

  async function loadStatus() {
    try {
      const data = await readJson(await fetch("/api/copilot/status"));
      const items = [];
      if (data.ai && data.ai.configured) {
        items.push(statusItem("ok", `AI: ${data.ai.label}`));
      } else {
        items.push(statusItem("off", "AI is off: add XAI_API_KEY or OPENAI_API_KEY to .env and restart the server"));
        input.disabled = true;
        submitButton.disabled = true;
        input.placeholder = "Configure an AI key on the server to chat";
      }
      const paypal = data.paypal || {};
      if (paypal.connected) {
        items.push(statusItem("ok", "PayPal Sandbox: orders, payment links & refunds"));
        items.push(statusItem(paypal.transactionSearch ? "ok" : "off", paypal.transactionSearch ? "Transaction Search" : "Transaction Search not enabled"));
        items.push(statusItem(paypal.invoicing ? "ok" : "off", paypal.invoicing ? "Invoicing" : "Invoicing not enabled"));
        if (!paypal.transactionSearch || !paypal.invoicing) {
          items.push(statusItem("hint", "To enable: Developer Dashboard → Apps & Credentials → your Sandbox app → Features"));
        }
      } else {
        items.push(statusItem("off", `PayPal Sandbox: ${paypal.error || "not connected"}`));
      }
      statusList.replaceChildren(...items);
      badge.textContent = data.ai && data.ai.configured ? "LIVE AI · SANDBOX" : "AI NOT CONFIGURED";
    } catch (error) {
      statusList.replaceChildren(statusItem("off", `Could not check connections: ${error.message} Open the app through its local server.`));
      badge.textContent = "OFFLINE";
    }
  }

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    return ask(input.value);
  });

  suggestions.addEventListener("click", (event) => {
    const chip = event.target.closest("[data-prompt]");
    if (!chip || busy || input.disabled) return undefined;
    return ask(chip.dataset.prompt);
  });

  // The Boardroom page hands its first step to the Copilot; the merchant still approves it.
  if (typeof document.addEventListener === "function") {
    document.addEventListener("boardroom:ask", (event) => {
      const prompt = event.detail && event.detail.prompt;
      if (!prompt) return;
      if (input.disabled) {
        addMessage(BOT_NAME, "The AI is off, so I can't draft this step. Add an AI key on the server and restart.", "ai-advisor ai-error");
        return;
      }
      ask(prompt);
    });
  }

  loadStatus();
})();
