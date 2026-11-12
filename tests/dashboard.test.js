"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { all, createPage, flush, response, text } = require("./fake-dom");

const source = fs.readFileSync(path.join(__dirname, "../public/dashboard.js"), "utf8");

function dashboard(days = 30) {
  return {
    days,
    hasData: true,
    gross: { cents: 18000, text: "$180.00", changePct: 80 },
    orders: { count: 4, previous: 1 },
    averageOrder: { cents: 4500, text: "$45.00", changePct: -55 },
    repeatCustomers: { pct: 33.3, customers: 3, repeat: 1, changePts: null },
    refunds: { cents: 500, text: "$5.00", pctOfGross: 2.8 },
    net: { cents: 17500, text: "$175.00" },
    unpaid: { count: 0, cents: 0, text: "$0.00" },
    series: Array.from({ length: days }, (_, index) => ({ date: `2026-11-${String((index % 28) + 1).padStart(2, "0")}`, cents: index === days - 1 ? 3000 : 0 })),
    buckets: Array.from({ length: 12 }, (_, index) => ({ gross: index * 100, orders: index % 2, refunds: 0 })),
    topItems: [{ name: "Tote", cents: 9000, text: "$90.00", orders: 2 }],
    insight: { title: "Sales are up 80%", detail: "$180.00 now vs. $100.00 before.", tag: "GROWTH" },
  };
}

const ACTIVITY = {
  orders: [{
    id: "ORDER0000000001", createdAt: "2026-11-08T10:00:00Z", capturedAt: "2026-11-08T10:05:00Z", source: "request",
    description: "Everyday Tote", payer: "Avery Chen", amount: "$45.00", captured: "$45.00", refunded: "$0.00",
    refundableCents: 4500, refundable: "45.00", status: "Paid", state: "captured", approvalUrl: null,
  }],
  payouts: [],
  invoices: [],
  events: [{ at: "2026-11-08T10:05:00Z", kind: "payment", title: "Payment captured", detail: "$45.00 · Everyday Tote" }],
};

const ROUTES = {
  "/api/profile": () => response(200, { displayName: "Avery", businessName: "Avery's Bags" }),
  "/api/dashboard?days=30": () => response(200, dashboard(30)),
  "/api/dashboard?days=7": () => response(200, dashboard(7)),
  "/api/activity": () => response(200, ACTIVITY),
  "/api/wallet": () => response(200, {
    boardroom: { captured: "$180.00", refunded: "$5.00", paidOut: "$0.00", net: "$175.00", payments: 4 },
    paypal: { available: false, error: "Showing the PayPal account balance needs the Transaction search feature." },
  }),
};

test("dashboard figures come from the server and follow the selected period", async () => {
  const { $, calls } = await createPage(source, ROUTES);
  assert.equal($("#metric-gross").textContent, "$180.00");
  assert.equal($("#metric-gross-trend").textContent, "↑ 80%");
  assert.ok($("#metric-gross-trend").classList.contains("up"));
  assert.equal($("#metric-aov-trend").textContent, "↓ 55%");
  assert.ok($("#metric-aov-trend").classList.contains("down"));
  assert.equal($("#metric-repeat").textContent, "33.3%");
  assert.equal($("#metric-repeat-note").textContent, "1 of 3 customers");
  assert.equal($("#insight-tag").textContent, "GROWTH");
  assert.equal($("#demo-display-name").textContent, "Avery");
  assert.equal($("#avatar").textContent, "A");
  assert.equal($("#spark-gross").children.length, 12);
  assert.match($("#chart-line").attributes.d, /^M0\.0 /);
  assert.equal($("#chart-total").textContent, "$180.00");
  assert.match(text($("#top-items")), /Tote/);
  assert.match(text($("#audit-list")), /Payment captured/);

  $("#dashboard-range").value = "7";
  await $("#dashboard-range").listeners.change();
  await flush();
  assert.ok(calls.some((call) => call.url === "/api/dashboard?days=7"));
  assert.equal($("#dashboard-range-label").textContent, "LAST 7 DAYS");
});

test("a refund is requested only after the merchant confirms an amount", async () => {
  const { $, calls } = await createPage(source, {
    ...ROUTES,
    "POST /api/orders/ORDER0000000001/refund": () => response(200, { refundId: "REFUND000000001", status: "COMPLETED", amount: "5.00 USD" }),
  });
  const refund = all($("#activity-list"), (child) => child.tag === "button" && child.textContent === "Refund")[0];
  assert.ok(refund, "captured orders with a balance offer a refund");
  await refund.listeners.click();
  const form = all($("#activity-list"), (child) => child.tag === "form")[0];
  const input = all(form, (child) => child.tag === "input")[0];
  assert.equal(input.value, "45.00");
  assert.ok(!calls.some((call) => call.method === "POST"), "opening the form must not refund");

  input.value = "50";
  await form.listeners.submit({ preventDefault() {} });
  assert.ok(!calls.some((call) => call.method === "POST"), "over-balance refunds are refused in the page");

  input.value = "5";
  await form.listeners.submit({ preventDefault() {} });
  await flush();
  const post = calls.find((call) => call.method === "POST");
  assert.equal(post.body.amount, "5.00");
  assert.match(post.body.requestId, /^[0-9a-f-]{36}$/);
  assert.match(text(form), /REFUND000000001: COMPLETED/);
});

test("the wallet falls back to the net through Boardroom and says why", async () => {
  const { $ } = await createPage(source, ROUTES);
  assert.equal($("#wallet-card-label").textContent, "NET THROUGH BOARDROOM");
  assert.equal($("#wallet-card-value").textContent, "$175.00");
  assert.match($("#wallet-paypal-note").textContent, /Transaction search/);
  assert.match(text($("#wallet-rows")), /Sent with Payouts/);
});

test("an empty account shows how to get started instead of sample data", async () => {
  const empty = { ...dashboard(30), hasData: false, gross: { cents: 0, text: "$0.00", changePct: null }, orders: { count: 0, previous: 0 }, topItems: [] };
  const { $ } = await createPage(source, {
    ...ROUTES,
    "/api/dashboard?days=30": () => response(200, empty),
    "/api/activity": () => response(200, { orders: [], payouts: [], invoices: [], events: [] }),
  });
  assert.match($("#dashboard-source").textContent, /No payments captured yet/);
  assert.match(text($("#activity-list")), /Nothing yet/);
  assert.match(text($("#top-items")), /No paid items/);
  assert.equal($("#audit-empty").hidden, false);
});
