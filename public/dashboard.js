"use strict";

// Dashboard, sales snapshot, activity, audit trail, and wallet. Every figure comes from the
// server's record of real PayPal Sandbox activity; nothing on these pages is sample data.
(() => {
  const $ = (selector) => document.querySelector(selector);
  const rangeSelect = $("#dashboard-range");
  let days = 30;
  let loading = null;

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined && text !== null) element.textContent = String(text);
    return element;
  }

  async function getJson(url) {
    const response = await fetch(url, { headers: { Accept: "application/json" } });
    let data = {};
    try { data = await response.json(); } catch { data = {}; }
    if (!response.ok) throw new Error(data.error || `Request failed (HTTP ${response.status}).`);
    return data;
  }

  async function postJson(url, body) {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body || {}),
    });
    let data = {};
    try { data = await response.json(); } catch { data = {}; }
    if (!response.ok) throw new Error(data.error || `Request failed (HTTP ${response.status}).`);
    return data;
  }

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

  function when(iso) {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? "" : date.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  }

  function shortDate(isoDate) {
    const date = new Date(`${isoDate}T12:00:00Z`);
    return Number.isNaN(date.getTime()) ? isoDate : date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }

  function dollars(cents) {
    const value = Math.abs(cents) / 100;
    if (value >= 1000) return `$${(value / 1000).toFixed(value >= 10000 ? 0 : 1)}k`;
    return `$${Math.round(value)}`;
  }

  // ---------------------------------------------------------------------------------------------
  // Dashboard

  function setTrend(id, value, { suffix = "%", invert = false, fallback = "" } = {}) {
    const element = $(id);
    element.classList.remove("up", "down", "neutral");
    if (value === null || value === undefined) {
      element.textContent = fallback;
      element.classList.add("neutral");
      return;
    }
    const good = invert ? value < 0 : value > 0;
    element.classList.add(value === 0 ? "neutral" : good ? "up" : "down");
    element.textContent = `${value > 0 ? "↑" : value < 0 ? "↓" : "→"} ${Math.abs(value)}${suffix}`;
  }

  function sparkline(id, values) {
    const element = $(id);
    const max = Math.max(...values, 0);
    element.replaceChildren(...values.map((value) => {
      const bar = node("i");
      bar.style.height = `${max > 0 ? Math.max(6, Math.round((value / max) * 100)) : 6}%`;
      return bar;
    }));
  }

  function renderChart(series, totalText) {
    const values = series.map((day) => day.cents);
    const max = Math.max(...values, 0);
    const top = max > 0 ? Math.ceil(max / 100 / 3) * 3 * 100 : 300;
    const labels = $("#chart-y-labels");
    labels.replaceChildren(...[1, 2 / 3, 1 / 3, 0].map((share) => node("span", "", dollars(top * share))));
    const step = values.length > 1 ? 360 / (values.length - 1) : 360;
    const points = values.map((cents, index) => `${(index * step).toFixed(1)} ${(146 - (cents / top) * 140).toFixed(1)}`);
    const line = `M${points.join(" L")}`;
    $("#chart-line").setAttribute("d", line);
    $("#chart-fill").setAttribute("d", `${line} L360 150 L0 150Z`);
    const xLabels = $("#chart-x-labels");
    const picks = [0, 0.25, 0.5, 0.75, 1].map((share) => series[Math.round(share * (series.length - 1))]);
    xLabels.replaceChildren(...picks.map((day) => node("span", "", day ? shortDate(day.date) : "")));
    $("#chart-total").textContent = totalText;
    $("#snapshot-chart").setAttribute("aria-label", `Daily gross sales over the last ${series.length} days, ${totalText} in total`);
  }

  function renderTopItems(items) {
    const list = $("#top-items");
    if (!items.length) {
      list.replaceChildren(node("p", "data-caption", "No paid items in this period yet."));
      return;
    }
    list.replaceChildren(...items.map((item, index) => {
      const row = node("div", "product-row");
      row.append(node("span", "rank", String(index + 1).padStart(2, "0")), node("span", "", item.name), node("strong", "", item.text));
      return row;
    }));
  }

  function renderDashboard(data) {
    const period = `vs. previous ${data.days} days`;
    $("#dashboard-range-label").textContent = `LAST ${data.days} DAYS`;
    $("#dashboard-source").textContent = data.hasData
      ? `${data.orders.count} paid order${data.orders.count === 1 ? "" : "s"} in this period, from payments captured through Boardroom on PayPal Sandbox.`
      : "No payments captured yet. Request money or share a Copilot payment link to get started.";

    $("#metric-gross").textContent = data.gross.text;
    setTrend("#metric-gross-trend", data.gross.changePct, { fallback: data.orders.count && !data.orders.previous ? "New" : "" });
    $("#metric-gross-note").textContent = data.gross.changePct === null ? `${data.orders.count} paid orders` : period;

    $("#metric-aov").textContent = data.averageOrder.text || "—";
    setTrend("#metric-aov-trend", data.averageOrder.changePct);
    $("#metric-aov-note").textContent = data.averageOrder.changePct === null ? "per paid order" : period;

    const repeat = data.repeatCustomers;
    $("#metric-repeat").textContent = repeat.pct === null ? "—" : `${repeat.pct}%`;
    setTrend("#metric-repeat-trend", repeat.changePts, { suffix: " pts" });
    $("#metric-repeat-note").textContent = repeat.changePts === null
      ? `${repeat.repeat} of ${repeat.customers} customers`
      : period;

    $("#metric-refunds").textContent = data.refunds.text;
    const refundTrend = $("#metric-refunds-trend");
    refundTrend.classList.remove("up", "down");
    refundTrend.classList.add("neutral");
    refundTrend.textContent = data.refunds.pctOfGross === null ? "" : `${data.refunds.pctOfGross}%`;
    $("#metric-refunds-note").textContent = data.refunds.pctOfGross === null ? "refunded" : "of gross sales";

    sparkline("#spark-gross", data.buckets.map((bucket) => bucket.gross));
    sparkline("#spark-orders", data.buckets.map((bucket) => (bucket.orders ? bucket.gross / bucket.orders : 0)));
    sparkline("#spark-customers", data.buckets.map((bucket) => bucket.orders));
    sparkline("#spark-refunds", data.buckets.map((bucket) => bucket.refunds));

    $("#insight-title").textContent = data.insight.title;
    $("#insight-detail").textContent = data.insight.detail;
    $("#insight-tag").textContent = data.insight.tag;

    renderChart(data.series, data.gross.text);
    renderTopItems(data.topItems);
    $("#snapshot-caption").textContent = `Last ${data.days} days · payments captured through Boardroom on PayPal Sandbox`;
  }

  function renderProfile(profile) {
    const name = profile.displayName || "there";
    const hour = new Date().getHours();
    $("#dashboard-greeting").textContent = hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
    $("#demo-display-name").textContent = name;
    const avatar = $("#avatar");
    avatar.textContent = name.trim().charAt(0).toUpperCase() || "M";
    avatar.setAttribute("aria-label", `${name}: profile and settings`);
  }

  // ---------------------------------------------------------------------------------------------
  // Activity

  const SOURCE_LABEL = {
    checkout: "Sandbox checkout",
    card: "Card payment",
    payment_link: "Copilot payment link",
    request: "Payment request",
  };

  function refundControls(order, container) {
    const open = node("button", "button button-secondary button-small", "Refund");
    open.type = "button";
    container.append(open);
    open.addEventListener("click", () => {
      open.hidden = true;
      const form = node("form", "inline-form");
      const label = node("label", "", `Refund amount (max $${order.refundable})`);
      const input = node("input");
      input.type = "number";
      input.min = "0.01";
      input.max = order.refundable;
      input.step = "0.01";
      input.value = order.refundable;
      input.id = `refund-${order.id}`;
      label.htmlFor = input.id;
      const confirm = node("button", "button button-primary button-small", "Confirm refund");
      confirm.type = "submit";
      const cancel = node("button", "button button-secondary button-small", "Cancel");
      cancel.type = "button";
      const message = node("small", "row-message");
      form.append(label, input, confirm, cancel, message);
      container.append(form);
      cancel.addEventListener("click", () => { form.remove(); open.hidden = false; });
      form.addEventListener("submit", async (event) => {
        event.preventDefault();
        const amount = Number(input.value);
        if (!Number.isFinite(amount) || amount <= 0 || amount > Number(order.refundable)) {
          message.textContent = `Enter an amount between $0.01 and $${order.refundable}.`;
          return;
        }
        confirm.disabled = true;
        cancel.disabled = true;
        message.textContent = "Asking PayPal to refund…";
        try {
          const result = await postJson(`/api/orders/${encodeURIComponent(order.id)}/refund`, { amount: amount.toFixed(2), requestId: requestId() });
          message.textContent = `PayPal refund ${result.refundId || ""}: ${result.status || "submitted"}.`;
          announceChange();
        } catch (error) {
          message.textContent = `${error.message}`;
          confirm.disabled = false;
          cancel.disabled = false;
        }
      });
      input.focus();
    });
  }

  function copyButton(url, container) {
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
    const open = node("a", "button button-secondary button-small", "Open on PayPal ↗");
    open.href = url;
    open.target = "_blank";
    open.rel = "noopener noreferrer";
    container.append(copy, open);
  }

  function orderRow(order) {
    const row = node("div", "activity-row");
    const label = node("span");
    const who = order.payer || order.customer;
    label.append(
      node("strong", "", order.description),
      node("small", "", [SOURCE_LABEL[order.source] || "Order", order.status, who, when(order.capturedAt || order.createdAt)].filter(Boolean).join(" · ")),
    );
    const actions = node("span", "row-actions");
    if (order.refundableCents > 0) refundControls(order, actions);
    if (order.approvalUrl) copyButton(order.approvalUrl, actions);
    if (actions.childNodes.length || actions.children.length) label.append(actions);
    const amount = order.state === "captured"
      ? `+${order.captured}${order.refunded !== "$0.00" ? ` (−${order.refunded})` : ""}`
      : order.amount;
    row.append(label, node("strong", order.state === "captured" ? "amount-in" : "amount-pending", amount));
    return { at: order.capturedAt || order.createdAt, row };
  }

  function payoutRow(payout) {
    const row = node("div", "activity-row");
    const label = node("span");
    label.append(
      node("strong", "", `Payout to ${payout.recipient}`),
      node("small", "", ["PayPal Payouts", payout.status, when(payout.createdAt)].join(" · ")),
    );
    const actions = node("span", "row-actions");
    const check = node("button", "button button-secondary button-small", "Check status");
    check.type = "button";
    const message = node("small", "row-message");
    check.addEventListener("click", async () => {
      check.disabled = true;
      message.textContent = "Checking with PayPal…";
      try {
        const result = await getJson(`/api/payouts/${encodeURIComponent(payout.batchId)}`);
        message.textContent = `PayPal status: ${result.status}.`;
        if (result.status !== payout.status) announceChange();
      } catch (error) {
        message.textContent = error.message;
      } finally {
        check.disabled = false;
      }
    });
    actions.append(check, message);
    label.append(actions);
    row.append(label, node("strong", "amount-out", `−${payout.amount}`));
    return { at: payout.createdAt, row };
  }

  function invoiceRow(invoice) {
    const row = node("div", "activity-row");
    const label = node("span");
    label.append(
      node("strong", "", `Invoice to ${invoice.recipient}`),
      node("small", "", ["PayPal invoice", invoice.status.replace(/_/g, " ").toLowerCase(), invoice.id, when(invoice.createdAt)].join(" · ")),
    );
    const link = node("a", "workspace-inline-link row-link", "Manage in Invoicing →");
    link.href = "#invoicing";
    label.append(link);
    row.append(label, node("strong", "amount-pending", invoice.amount));
    return { at: invoice.createdAt, row };
  }

  function renderActivity(data) {
    const list = $("#activity-list");
    const rows = [
      ...data.orders.map(orderRow),
      ...data.payouts.map(payoutRow),
      ...data.invoices.map(invoiceRow),
    ].sort((a, b) => String(b.at).localeCompare(String(a.at)));
    if (!rows.length) {
      list.replaceChildren(node("p", "empty-state", "Nothing yet. Request money, send a payout, create an invoice, or try the Sandbox checkout, and it appears here."));
    } else {
      list.replaceChildren(...rows.slice(0, 60).map((item) => item.row));
    }

    const audit = $("#audit-list");
    audit.replaceChildren(...data.events.slice(0, 40).map((event) => {
      const item = node("li");
      const time = node("time", "", when(event.at));
      time.dateTime = event.at;
      item.append(node("strong", "", event.title), node("span", "", event.detail), time);
      return item;
    }));
    $("#audit-empty").hidden = data.events.length > 0;
  }

  // ---------------------------------------------------------------------------------------------
  // Wallet

  function walletRow(label, value) {
    const row = node("div");
    row.append(node("span", "", label), node("strong", "", value));
    return row;
  }

  function renderWallet(data) {
    const boardroom = data.boardroom;
    const paypal = data.paypal || {};
    if (paypal.available) {
      $("#wallet-card-label").textContent = "PAYPAL SANDBOX BALANCE";
      $("#wallet-card-value").textContent = paypal.availableBalance || paypal.total || "—";
      $("#wallet-card-note").textContent = `Available · total ${paypal.total || "—"}${paypal.asOf ? ` · as of ${when(paypal.asOf)}` : ""}`;
      $("#wallet-paypal-note").textContent = "";
    } else {
      $("#wallet-card-label").textContent = "NET THROUGH BOARDROOM";
      $("#wallet-card-value").textContent = boardroom.net;
      $("#wallet-card-note").textContent = `${boardroom.payments} paid order${boardroom.payments === 1 ? "" : "s"} · Sandbox test money`;
      $("#wallet-paypal-note").textContent = paypal.error || "";
    }
    const minus = (text) => (text === "$0.00" ? text : `−${text}`);
    $("#wallet-rows").replaceChildren(
      walletRow("Payments captured", boardroom.captured),
      walletRow("Refunded", minus(boardroom.refunded)),
      walletRow("Sent with Payouts", minus(boardroom.paidOut)),
      walletRow("Net through Boardroom", boardroom.net),
    );
  }

  // ---------------------------------------------------------------------------------------------

  function showError(error) {
    $("#dashboard-source").textContent = `${error.message} Open the app through its server (cargo run).`;
    $("#insight-title").textContent = "Could not load your data";
    $("#insight-detail").textContent = error.message;
  }

  async function refresh() {
    if (loading) return loading;
    loading = (async () => {
      const results = await Promise.allSettled([
        getJson("/api/profile"),
        getJson(`/api/dashboard?days=${days}`),
        getJson("/api/activity"),
        getJson("/api/wallet"),
      ]);
      const [profile, dashboard, activity, wallet] = results;
      if (profile.status === "fulfilled") renderProfile(profile.value);
      if (dashboard.status === "fulfilled") renderDashboard(dashboard.value);
      else showError(dashboard.reason);
      if (activity.status === "fulfilled") renderActivity(activity.value);
      else $("#activity-list").replaceChildren(node("p", "empty-state", activity.reason.message));
      if (wallet.status === "fulfilled") renderWallet(wallet.value);
      else $("#wallet-card-note").textContent = wallet.reason.message;
    })();
    try {
      await loading;
    } finally {
      loading = null;
    }
  }

  rangeSelect.addEventListener("change", () => {
    days = Number(rangeSelect.value) || 30;
    refresh();
  });
  if (typeof document.addEventListener === "function") {
    document.addEventListener("boardroom:changed", () => refresh());
  }

  // Account-wide history from Transaction Search (needs the Sandbox app's permission).
  const sandboxActivityButton = $("#sandbox-activity-load");
  sandboxActivityButton.addEventListener("click", async () => {
    const status = $("#sandbox-activity-status");
    const results = $("#sandbox-activity-results");
    sandboxActivityButton.disabled = true;
    status.textContent = "Loading transactions from PayPal…";
    results.replaceChildren();
    const end = new Date();
    const start = new Date(end.getTime() - 30 * 24 * 60 * 60 * 1000);
    const stamp = (date) => date.toISOString().replace(/\.\d{3}Z$/, "Z");
    const query = new URLSearchParams({ start: stamp(start), end: stamp(end) });
    try {
      const data = await getJson(`/api/sandbox/transactions?${query}`);
      for (const transaction of data.transactions) {
        const row = node("div");
        const label = node("span");
        label.append(
          node("strong", "", transaction.id || "PayPal transaction"),
          node("small", "", `${transaction.date ? when(transaction.date) : "Date unavailable"} · ${transaction.status || "Status unavailable"}`),
        );
        const amount = transaction.amount
          ? `${transaction.amount.currency_code || ""} ${transaction.amount.value || ""}`.trim()
          : "Amount unavailable";
        row.append(label, node("strong", "", amount));
        results.append(row);
      }
      status.textContent = data.transactions.length
        ? `Showing ${data.transactions.length} transaction(s), first page only. PayPal reporting can lag by up to three hours.`
        : "PayPal returned no transactions for the last 30 days. Reporting can lag by up to three hours.";
    } catch (error) {
      status.textContent = error.message;
    } finally {
      sandboxActivityButton.disabled = false;
    }
  });

  refresh();
})();
