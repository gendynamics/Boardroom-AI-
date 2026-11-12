"use strict";

// Send & request, Invoicing, and Settings. Each action is reviewed in the page first; only the
// explicit Confirm / Create / Send click asks the server to call PayPal Sandbox.
(() => {
  const $ = (selector) => document.querySelector(selector);
  const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });
  const TEST_EMAIL = /^[a-z0-9_-]+(?:\.[a-z0-9_-]+)*@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)*example\.com$/;

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined && text !== null) element.textContent = String(text);
    return element;
  }

  async function readJson(response) {
    let data = {};
    try { data = await response.json(); } catch { data = {}; }
    if (!response.ok) throw new Error(data.error || `Request failed (HTTP ${response.status}).`);
    return data;
  }

  const getJson = async (url) => readJson(await fetch(url, { headers: { Accept: "application/json" } }));
  const postJson = async (url, body) => readJson(await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body || {}),
  }));

  function requestId() {
    if (globalThis.crypto && typeof globalThis.crypto.randomUUID === "function") return globalThis.crypto.randomUUID();
    const hex = Array.from({ length: 32 }, () => Math.floor(Math.random() * 16).toString(16));
    hex[12] = "4";
    hex[16] = ((parseInt(hex[16], 16) & 3) | 8).toString(16);
    const text = hex.join("");
    return `${text.slice(0, 8)}-${text.slice(8, 12)}-${text.slice(12, 16)}-${text.slice(16, 20)}-${text.slice(20)}`;
  }

  function announceChange() {
    if (typeof CustomEvent === "function" && typeof document.dispatchEvent === "function") {
      document.dispatchEvent(new CustomEvent("boardroom:changed"));
    }
  }

  function isPayPalUrl(url) {
    return typeof url === "string" && /^https:\/\/([a-z0-9-]+\.)*paypal\.com\//i.test(url);
  }

  function detailRow(label, value) {
    const row = node("div");
    row.append(node("span", "", label), node("strong", "", value));
    return row;
  }

  function linkParts(url, text) {
    const link = node("a", "button button-primary button-small", text);
    link.href = url;
    link.target = "_blank";
    link.rel = "noopener noreferrer";
    const copy = node("button", "button button-secondary button-small", "Copy link");
    copy.type = "button";
    copy.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(url);
        copy.textContent = "Copied";
      } catch {
        copy.textContent = "Copy failed";
      }
    });
    return [link, copy];
  }

  // ---------------------------------------------------------------------------------------------
  // Send & request

  const paymentForm = $("#payment-form");
  const paymentAction = $("#payment-action");
  const party = $("#payment-party");
  const partyLabel = $("#payment-party-label");
  const memo = $("#payment-memo");
  const memoLabel = $("#payment-memo-label");
  const review = $("#payment-review");
  const reviewDetails = $("#payment-review-details");
  const reviewNote = $("#payment-review-note");
  const confirmButton = $("#payment-confirm");
  const editButton = $("#payment-cancel");
  const paymentResult = $("#payment-result");
  let pending = null;

  function syncPaymentLabels() {
    const sending = paymentAction.value === "send";
    partyLabel.textContent = sending ? "Recipient email (Sandbox account at example.com)" : "Customer (optional, for your records)";
    party.placeholder = sending ? "sb-abc123@personal.example.com" : "Jordan Lee";
    party.type = sending ? "email" : "text";
    memoLabel.textContent = sending ? "Note to recipient (optional)" : "What it’s for (shown on PayPal)";
    memo.placeholder = sending ? "Thanks for your help this month" : "Everyday Tote";
  }

  function paymentMessage(text, extras = []) {
    const message = node("p", "workspace-note", text);
    paymentResult.replaceChildren(message, ...extras);
  }

  paymentAction.addEventListener("change", () => {
    syncPaymentLabels();
    paymentResult.replaceChildren();
  });

  paymentForm.addEventListener("submit", (event) => {
    event.preventDefault();
    const sending = paymentAction.value === "send";
    const who = party.value.trim();
    const note = memo.value.trim();
    const amount = Number(paymentForm.querySelector("#payment-amount").value);
    if (!Number.isFinite(amount) || amount < 0.01 || amount > 9999.99) {
      paymentMessage("Enter an amount between $0.01 and $9,999.99.");
      return;
    }
    if (sending && !TEST_EMAIL.test(who.toLowerCase())) {
      paymentMessage("Send only to a test address at example.com, such as a Sandbox buyer account (sb-…@personal.example.com).");
      return;
    }
    if (!sending && !note) {
      paymentMessage("Say what the payment is for. PayPal shows it to the buyer.");
      return;
    }
    pending = sending
      ? { kind: "send", body: { recipient: who.toLowerCase(), amount: amount.toFixed(2), note: note || undefined, requestId: requestId() } }
      : { kind: "request", body: { amount: amount.toFixed(2), description: note, customer: who || undefined } };
    reviewDetails.replaceChildren(
      detailRow("Action", sending ? "Send money with PayPal Payouts" : "Request money with a PayPal checkout link"),
      detailRow(sending ? "Recipient" : "Customer", who || "Not specified"),
      detailRow("Amount", `${usd.format(amount)} USD`),
      detailRow(sending ? "Note" : "For", note || "None"),
      detailRow("Money", "PayPal Sandbox test money"),
    );
    reviewNote.textContent = sending
      ? "Confirming sends this payout from your Sandbox business account right away. It cannot be undone from here."
      : "Confirming creates a PayPal order and a link to share. The buyer pays on PayPal; you can follow it in Activity.";
    confirmButton.textContent = sending ? `Send ${usd.format(amount)}` : "Create payment link";
    paymentForm.hidden = true;
    review.hidden = false;
    paymentResult.replaceChildren();
    $("#payment-review-heading").focus();
  });

  editButton.addEventListener("click", () => {
    pending = null;
    review.hidden = true;
    paymentForm.hidden = false;
    paymentMessage("Nothing was sent to PayPal.");
  });

  confirmButton.addEventListener("click", async () => {
    if (!pending) return;
    const current = pending;
    confirmButton.disabled = true;
    editButton.disabled = true;
    paymentMessage(current.kind === "send" ? "Sending the payout through PayPal…" : "Creating the PayPal order…");
    try {
      if (current.kind === "send") {
        const result = await postJson("/api/payouts", current.body);
        paymentMessage(`PayPal accepted a ${result.amount} payout to ${result.recipient} (batch ${result.batchId}, ${result.status}). Check its status in Activity.`);
      } else {
        const result = await postJson("/api/payments/request", current.body);
        const parts = isPayPalUrl(result.approvalUrl) ? linkParts(result.approvalUrl, "Open PayPal checkout ↗") : [];
        paymentMessage(`Payment link for ${result.amount} created (order ${result.orderId}). Share it with your customer. When they pay, the capture appears on your dashboard.`, parts);
      }
      pending = null;
      review.hidden = true;
      paymentForm.hidden = false;
      paymentForm.reset();
      syncPaymentLabels();
      announceChange();
    } catch (error) {
      paymentMessage(`${error.message} Nothing was recorded as sent. Check Sandbox activity before trying again.`);
    } finally {
      confirmButton.disabled = false;
      editButton.disabled = false;
    }
  });

  syncPaymentLabels();

  // ---------------------------------------------------------------------------------------------
  // Invoicing

  const invoiceForm = $("#invoice-form");
  const invoiceStatus = $("#invoice-status");
  const invoiceList = $("#invoice-list");
  const invoiceGate = $("#invoice-gate");
  const INVOICE_STATUS = {
    DRAFT: "Draft", SENT: "Sent", SCHEDULED: "Scheduled", PAID: "Paid", MARKED_AS_PAID: "Marked paid",
    CANCELLED: "Cancelled", REFUNDED: "Refunded", PARTIALLY_PAID: "Partially paid",
    PARTIALLY_REFUNDED: "Partially refunded", MARKED_AS_REFUNDED: "Marked refunded",
    UNPAID: "Unpaid", PAYMENT_PENDING: "Payment pending",
  };

  function setInvoicingEnabled(enabled, message) {
    invoiceGate.hidden = enabled;
    invoiceGate.textContent = enabled ? "" : message;
    for (const control of invoiceForm.querySelectorAll("input, select, button")) control.disabled = !enabled;
    $("#invoice-badge").textContent = enabled ? "PAYPAL SANDBOX" : "NEEDS PERMISSION";
  }

  function invoiceButton(label, invoice, action, confirmText) {
    const button = node("button", `button ${action === "send" ? "button-primary" : "button-secondary"} button-small`, label);
    button.type = "button";
    button.addEventListener("click", async () => {
      if (confirmText && typeof confirm === "function" && !confirm(confirmText)) return;
      button.disabled = true;
      invoiceStatus.textContent = `${label}…`;
      try {
        const result = await postJson(`/api/invoices/${encodeURIComponent(invoice.id)}/${action}`, {});
        invoiceStatus.textContent = action === "send"
          ? `Invoice ${invoice.id} sent. PayPal emails the customer in Sandbox; delivery is not confirmed.`
          : action === "remind" ? `Reminder sent for ${invoice.id}.` : `Invoice ${invoice.id} cancelled.`;
        if (action === "send" && isPayPalUrl(result.payUrl)) {
          invoiceStatus.append(" ", ...linkParts(result.payUrl, "Open customer view ↗"));
        }
        announceChange();
        loadInvoices();
      } catch (error) {
        invoiceStatus.textContent = `${error.message} Check Sandbox before retrying.`;
        button.disabled = false;
      }
    });
    return button;
  }

  function invoiceRow(invoice) {
    const row = node("div", "activity-row");
    const label = node("span");
    const detail = [INVOICE_STATUS[invoice.status] || invoice.status, invoice.number ? `#${invoice.number}` : invoice.id];
    if (invoice.dueDate) detail.push(`due ${invoice.dueDate}`);
    if (!invoice.managed) detail.push("created outside Boardroom");
    label.append(node("strong", "", invoice.recipient || "No recipient"), node("small", "", detail.join(" · ")));
    const actions = node("span", "row-actions");
    if (invoice.canSend) actions.append(invoiceButton("Send invoice", invoice, "send"));
    if (invoice.canRemind) actions.append(invoiceButton("Send reminder", invoice, "remind"));
    if (invoice.canCancel) actions.append(invoiceButton("Cancel", invoice, "cancel", `Cancel invoice ${invoice.id}? The customer is told it was cancelled.`));
    if (isPayPalUrl(invoice.payUrl) && invoice.status !== "DRAFT") {
      const view = node("a", "button button-secondary button-small", "Customer view ↗");
      view.href = invoice.payUrl;
      view.target = "_blank";
      view.rel = "noopener noreferrer";
      actions.append(view);
    }
    if (actions.children.length) label.append(actions);
    const due = invoice.due && invoice.due !== invoice.total && invoice.status !== "PAID" ? ` (${invoice.due} due)` : "";
    row.append(label, node("strong", "", `${invoice.total || "—"}${due}`));
    return row;
  }

  async function loadInvoices() {
    try {
      const data = await getJson("/api/invoices");
      if (!data.enabled) {
        setInvoicingEnabled(false, data.message);
        invoiceList.replaceChildren(node("p", "empty-state", "Invoices appear here once Invoicing is enabled for the Sandbox app."));
        return;
      }
      setInvoicingEnabled(true);
      invoiceList.replaceChildren(...(data.invoices.length
        ? data.invoices.map(invoiceRow)
        : [node("p", "empty-state", "No invoices on this Sandbox account yet.")]));
    } catch (error) {
      invoiceList.replaceChildren(node("p", "empty-state", error.message));
    }
  }

  invoiceForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const recipient = $("#invoice-recipient").value.trim().toLowerCase();
    const item = $("#invoice-item").value.trim();
    const quantity = Number($("#invoice-quantity").value);
    const price = Number($("#invoice-price").value);
    if (!TEST_EMAIL.test(recipient)) {
      invoiceStatus.textContent = "Use a test address at example.com, such as jordan@example.com or a Sandbox buyer account.";
      return;
    }
    if (!item || !Number.isFinite(quantity) || quantity <= 0 || !Number.isFinite(price) || price <= 0) {
      invoiceStatus.textContent = "Enter an item, a quantity, and a unit price.";
      return;
    }
    const total = Math.round(quantity * price * 100) / 100;
    if (total > 9999.99) {
      invoiceStatus.textContent = `The total would be ${usd.format(total)}. Keep invoices under $10,000.`;
      return;
    }
    const button = invoiceForm.querySelector("button");
    button.disabled = true;
    invoiceStatus.textContent = "Creating the draft on PayPal…";
    try {
      const result = await postJson("/api/invoices", {
        recipient,
        items: [{ name: item, quantity: String(quantity), unitPrice: price.toFixed(2), hours: $("#invoice-hours").checked }],
        paymentTerm: $("#invoice-term").value,
        note: $("#invoice-note").value.trim() || undefined,
      });
      invoiceStatus.textContent = `Draft ${result.invoiceId} for ${result.recipient}, ${result.total}, created on PayPal. Review it below, then send it.`;
      invoiceForm.reset();
      announceChange();
      loadInvoices();
    } catch (error) {
      invoiceStatus.textContent = `${error.message} Check Sandbox for a draft before retrying.`;
    } finally {
      button.disabled = invoiceGate.hidden === false;
    }
  });

  $("#invoice-refresh").addEventListener("click", () => loadInvoices());

  // ---------------------------------------------------------------------------------------------
  // Settings

  const profileForm = $("#profile-form");
  const profileResult = $("#profile-result");

  async function loadProfile() {
    try {
      const profile = await getJson("/api/profile");
      $("#profile-name").value = profile.displayName;
      $("#profile-business").value = profile.businessName;
    } catch (error) {
      profileResult.textContent = error.message;
    }
  }

  profileForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const displayName = $("#profile-name").value.trim();
    const businessName = $("#profile-business").value.trim();
    if (!displayName || !businessName) {
      profileResult.textContent = "Enter your name and a business name.";
      return;
    }
    const button = profileForm.querySelector("button");
    button.disabled = true;
    try {
      const saved = await postJson("/api/profile", { displayName, businessName });
      profileResult.textContent = `Saved. PayPal checkout now shows “${saved.businessName}”.`;
      announceChange();
    } catch (error) {
      profileResult.textContent = error.message;
    } finally {
      button.disabled = false;
    }
  });

  function chip(state, text) {
    return node("li", `status-${state}`, text);
  }

  async function loadConnections() {
    const list = $("#settings-status");
    try {
      const data = await getJson("/api/status");
      const paypal = data.paypal || {};
      const items = [];
      if (paypal.connected) {
        items.push(chip("ok", "PayPal Sandbox connected: orders & refunds"));
        items.push(chip(paypal.payouts ? "ok" : "off", paypal.payouts ? "Payouts" : "Payouts not enabled"));
        items.push(chip(paypal.invoicing ? "ok" : "off", paypal.invoicing ? "Invoicing" : "Invoicing not enabled"));
        items.push(chip(paypal.transactionSearch ? "ok" : "off", paypal.transactionSearch ? "Transaction Search" : "Transaction Search not enabled"));
        if (!paypal.payouts || !paypal.invoicing || !paypal.transactionSearch) {
          items.push(chip("hint", "To enable: Developer Dashboard → Apps & Credentials → your Sandbox app → Features"));
        }
      } else {
        items.push(chip("off", `PayPal Sandbox: ${paypal.error || "not connected"}`));
      }
      items.push(data.ai && data.ai.configured ? chip("ok", `AI: ${data.ai.label}`) : chip("off", "AI off: add XAI_API_KEY or OPENAI_API_KEY to .env"));
      list.replaceChildren(...items);
    } catch (error) {
      list.replaceChildren(chip("off", `Could not check connections: ${error.message}`));
    }
  }

  if (typeof document.addEventListener === "function") {
    document.addEventListener("boardroom:changed", () => loadInvoices());
  }

  loadProfile();
  loadConnections();
  loadInvoices();
})();
