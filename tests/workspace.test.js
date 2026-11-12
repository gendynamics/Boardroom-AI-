"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { all, createPage, flush, response, text } = require("./fake-dom");

const source = fs.readFileSync(path.join(__dirname, "../public/workspace.js"), "utf8");
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const BASE_ROUTES = {
  "/api/profile": () => response(200, { displayName: "Avery", businessName: "Avery's Bags" }),
  "/api/status": () => response(200, {
    ai: { configured: false },
    paypal: { connected: true, orders: true, invoicing: false, transactionSearch: false, payouts: true },
  }),
  "/api/invoices": () => response(200, { enabled: true, invoices: [] }),
};

async function page(routes = {}) {
  return createPage(source, { ...BASE_ROUTES, ...routes });
}

async function submit(form) {
  await form.listeners.submit({ preventDefault() {} });
  await flush();
}

test("a payment request is created only after review and confirmation", async () => {
  const { $, calls } = await page({
    "POST /api/payments/request": () => response(201, {
      orderId: "ORDER0000000001",
      approvalUrl: "https://www.sandbox.paypal.com/checkoutnow?token=ORDER0000000001",
      amount: "$45.00",
    }),
  });
  $("#payment-action").value = "request";
  $("#payment-party").value = "Jordan";
  $("#payment-amount").value = "45";
  $("#payment-memo").value = "Everyday Tote";

  await submit($("#payment-form"));
  assert.equal($("#payment-review").hidden, false);
  assert.equal($("#payment-confirm").textContent, "Create payment link");
  assert.ok(!calls.some((call) => call.method === "POST"), "review must not call the server");

  await $("#payment-confirm").listeners.click();
  await flush();
  const post = calls.find((call) => call.url === "/api/payments/request");
  assert.deepEqual(post.body, { amount: "45.00", description: "Everyday Tote", customer: "Jordan" });
  const link = all($("#payment-result"), (child) => child.tag === "a")[0];
  assert.equal(link.href, "https://www.sandbox.paypal.com/checkoutnow?token=ORDER0000000001");
  assert.equal($("#payment-review").hidden, true);
});

test("payouts go only to example.com test addresses and carry a fresh request ID", async () => {
  const { $, calls } = await page({
    "POST /api/payouts": (body) => response(201, { batchId: "BATCH123456", status: "PENDING", amount: "$12.50", recipient: body.recipient }),
  });
  $("#payment-action").value = "send";
  await $("#payment-action").listeners.change();
  assert.match($("#payment-party-label").textContent, /Recipient email/);
  $("#payment-amount").value = "12.5";

  $("#payment-review").hidden = true;
  $("#payment-party").value = "someone@gmail.com";
  await submit($("#payment-form"));
  assert.equal($("#payment-review").hidden, true, "an invalid recipient is never reviewed");
  assert.match(text($("#payment-result")), /test address at example\.com/);

  $("#payment-party").value = "SB-Buyer@personal.example.com";
  await submit($("#payment-form"));
  assert.equal($("#payment-review").hidden, false);
  assert.match(text($("#payment-review-note")), /cannot be undone/);
  await $("#payment-confirm").listeners.click();
  await flush();

  const posts = calls.filter((call) => call.url === "/api/payouts");
  assert.equal(posts.length, 1);
  assert.equal(posts[0].body.recipient, "sb-buyer@personal.example.com");
  assert.equal(posts[0].body.amount, "12.50");
  assert.match(posts[0].body.requestId, UUID);
  assert.match(text($("#payment-result")), /batch BATCH123456/);
});

test("editing a reviewed payment returns to the form without calling PayPal", async () => {
  const { $, calls } = await page();
  $("#payment-action").value = "request";
  $("#payment-amount").value = "10";
  $("#payment-memo").value = "Deposit";
  await submit($("#payment-form"));
  await $("#payment-cancel").listeners.click();
  assert.equal($("#payment-review").hidden, true);
  assert.equal($("#payment-form").hidden, false);
  assert.match(text($("#payment-result")), /Nothing was sent to PayPal/);
  assert.ok(!calls.some((call) => call.method === "POST"));
});

test("invoicing explains how to enable it when the Sandbox app lacks permission", async () => {
  const { $ } = await page({
    "/api/invoices": () => response(200, { enabled: false, message: "Invoicing is not enabled. In the PayPal Developer Dashboard, tick it.", invoices: [] }),
  });
  assert.equal($("#invoice-gate").hidden, false);
  assert.match($("#invoice-gate").textContent, /Developer Dashboard/);
  assert.equal($("#invoice-badge").textContent, "NEEDS PERMISSION");
});

test("an invoice draft is created from the form and listed invoices offer only permitted actions", async () => {
  const { $, calls } = await page({
    "/api/invoices": () => response(200, {
      enabled: true,
      invoices: [
        { id: "INV2-AAAA", status: "DRAFT", recipient: "jordan@example.com", total: "$205.00", managed: true, canSend: true, canRemind: false, canCancel: false },
        { id: "INV2-BBBB", status: "SENT", recipient: "sam@example.com", total: "$40.00", managed: false, canSend: false, canRemind: false, canCancel: false },
      ],
    }),
    "POST /api/invoices": () => response(201, { invoiceId: "INV2-AAAA", total: "$205.00", recipient: "jordan@example.com" }),
  });
  const buttons = all($("#invoice-list"), (child) => child.tag === "button").map((button) => button.textContent);
  assert.deepEqual(buttons, ["Send invoice"]);
  assert.match(text($("#invoice-list")), /created outside Boardroom/);

  $("#invoice-recipient").value = "jordan@gmail.com";
  $("#invoice-item").value = "Consulting";
  $("#invoice-quantity").value = "3";
  $("#invoice-price").value = "60";
  await submit($("#invoice-form"));
  assert.match($("#invoice-status").textContent, /test address at example\.com/);
  assert.ok(!calls.some((call) => call.method === "POST"));

  $("#invoice-recipient").value = "Jordan@Example.com";
  $("#invoice-hours").checked = true;
  $("#invoice-term").value = "NET_15";
  await submit($("#invoice-form"));
  const post = calls.find((call) => call.method === "POST" && call.url === "/api/invoices");
  assert.deepEqual(post.body, {
    recipient: "jordan@example.com",
    items: [{ name: "Consulting", quantity: "3", unitPrice: "60.00", hours: true }],
    paymentTerm: "NET_15",
  });
  assert.match($("#invoice-status").textContent, /Draft INV2-AAAA/);
});

test("settings load the saved profile and list missing PayPal permissions", async () => {
  const { $, calls } = await page({
    "POST /api/profile": (body) => response(200, body),
  });
  assert.equal($("#profile-name").value, "Avery");
  assert.equal($("#profile-business").value, "Avery's Bags");
  const chips = text($("#settings-status"));
  assert.match(chips, /Invoicing not enabled/);
  assert.match(chips, /Transaction Search not enabled/);
  assert.match(chips, /AI off/);

  $("#profile-business").value = "Avery & Co";
  await submit($("#profile-form"));
  assert.deepEqual(calls.find((call) => call.method === "POST").body, { displayName: "Avery", businessName: "Avery & Co" });
  assert.match($("#profile-result").textContent, /Avery & Co/);
});
