"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../public/app.js"), "utf8");

function createPage({ pathname = "/", search = "", fetch } = {}) {
  const elements = new Map();
  const navigations = [];
  const history = [];
  const document = {
    querySelector(selector) {
      if (!elements.has(selector)) {
        elements.set(selector, {
          hidden: selector === "#purchase-review",
          disabled: false,
          textContent: "",
          listeners: {},
          classList: { add() {}, remove() {} },
          addEventListener(event, callback) { this.listeners[event] = callback; },
          focus() { this.focused = true; },
        });
      }
      return elements.get(selector);
    },
  };
  const window = {
    location: { pathname, search, assign(url) { navigations.push(url); } },
    history: { replaceState(_state, _title, url) { history.push(url); } },
  };
  vm.runInNewContext(source, {
    document,
    window,
    URLSearchParams,
    fetch: fetch || (() => { throw new Error("Unexpected network request"); }),
  });
  return { elements, navigations, history };
}

test("review and cancellation do not create an order", async () => {
  const { elements } = createPage();
  const button = elements.get("#checkout-button");
  const review = elements.get("#purchase-review");
  const heading = elements.get("#purchase-review-heading");

  await button.listeners.click();
  assert.equal(review.hidden, false);
  assert.equal(button.hidden, true);
  assert.equal(heading.focused, true);

  await elements.get("#checkout-cancel-review").listeners.click();
  assert.equal(review.hidden, true);
  assert.equal(button.hidden, false);
  assert.equal(button.focused, true);
  assert.match(elements.get("#checkout-message").textContent, /No Sandbox order was created/);
});

test("only continuing the reviewed purchase requests an order", async () => {
  const calls = [];
  const { elements, navigations } = createPage({
    fetch: async (url, options) => {
      calls.push({ url, options });
      return {
        ok: true,
        json: async () => ({ approvalUrl: "https://www.sandbox.paypal.com/checkoutnow" }),
      };
    },
  });

  await elements.get("#checkout-button").listeners.click();
  assert.equal(calls.length, 0);
  await elements.get("#checkout-confirm").listeners.click();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "/api/checkout/orders");
  assert.equal(calls[0].options.method, "POST");
  assert.deepEqual(navigations, ["https://www.sandbox.paypal.com/checkoutnow"]);
  assert.equal(elements.get("#checkout-confirm").disabled, true);
});

test("order creation failure shows the error and allows retry or cancellation", async () => {
  const { elements } = createPage({
    fetch: async () => ({
      ok: false,
      json: async () => ({ error: "Sandbox unavailable" }),
    }),
  });
  await elements.get("#checkout-button").listeners.click();
  await elements.get("#checkout-confirm").listeners.click();
  assert.equal(elements.get("#checkout-message").textContent, "Sandbox unavailable");
  assert.equal(elements.get("#checkout-confirm").disabled, false);
  assert.equal(elements.get("#checkout-cancel-review").disabled, false);
});

test("cancelled checkout does not request capture", async () => {
  const { elements, history } = createPage({ pathname: "/checkout/cancel" });
  assert.match(elements.get("#checkout-message").textContent, /No capture was requested/);
  assert.deepEqual(history, ["/#sandbox-purchase"]);
});

test("return without an order ID reports the problem", async () => {
  const { elements, history } = createPage({ pathname: "/checkout/complete" });
  assert.match(elements.get("#checkout-message").textContent, /did not return an order ID/);
  assert.deepEqual(history, ["/#sandbox-purchase"]);
});

test("non-completed capture is not reported as completed", async () => {
  const calls = [];
  const { elements, history } = createPage({
    pathname: "/checkout/complete",
    search: "?token=5O190127TN364715T",
    fetch: async (url) => {
      calls.push(url);
      return { ok: true, json: async () => ({ status: "PENDING" }) };
    },
  });
  await new Promise(setImmediate);
  assert.deepEqual(calls, ["/api/checkout/orders/5O190127TN364715T/capture"]);
  assert.match(elements.get("#checkout-message").textContent, /status: PENDING/);
  assert.equal(elements.get("#checkout-button").disabled, false);
  assert.deepEqual(history, ["/#sandbox-purchase"]);
});
