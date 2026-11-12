"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../public/checkout-lab.js"), "utf8");

function createLab({ eligible = true, captureStatus = "COMPLETED", submissionFails = false } = {}) {
  const elements = new Map();
  const requests = [];
  const rendered = [];
  let callbacks;
  const makeField = (name) => ({
    render: async (target) => { rendered.push({ name, target }); },
  });
  const cardFields = {
    isEligible: () => eligible,
    NameField: () => makeField("name"),
    NumberField: () => makeField("number"),
    ExpiryField: () => makeField("expiry"),
    CVVField: () => makeField("cvv"),
    async submit() {
      if (submissionFails) throw new Error("Sandbox decline");
      const orderID = await callbacks.createOrder();
      await callbacks.onApprove({ orderID });
    },
  };
  const window = {
    paypal: {
      CardFields(options) {
        callbacks = options;
        return cardFields;
      },
    },
  };
  const document = {
    querySelector(selector) {
      if (!elements.has(selector)) {
        elements.set(selector, {
          value: selector === "#lab-scenario" ? "success" : "",
          hidden: selector === "#lab-fields",
          disabled: false,
          textContent: "",
          listeners: {},
          addEventListener(event, callback) { this.listeners[event] = callback; },
        });
      }
      return elements.get(selector);
    },
    createElement() { return {}; },
    head: { appendChild(script) { queueMicrotask(() => script.onload()); } },
  };
  const fetch = async (url, options) => {
    requests.push({ url, options });
    if (url === "/api/checkout/card/config") {
      return { ok: true, json: async () => ({ clientId: "sandbox_client_id" }) };
    }
    if (url === "/api/checkout/card/orders") {
      return { ok: true, json: async () => ({ orderId: "5O190127TN364715T" }) };
    }
    if (url === "/api/checkout/orders/5O190127TN364715T/capture") {
      return { ok: true, json: async () => ({ status: captureStatus }) };
    }
    throw new Error(`Unexpected request: ${url}`);
  };
  vm.runInNewContext(source, {
    document,
    window,
    fetch,
    URLSearchParams,
    console: { error() {} },
  });
  return { elements, requests, rendered };
}

test("ineligible accounts cannot submit card orders", async () => {
  const { elements, requests, rendered } = createLab({ eligible: false });
  await elements.get("#lab-load").listeners.click();
  assert.equal(elements.get("#lab-fields").hidden, true);
  assert.match(elements.get("#lab-message").textContent, /unavailable/);
  assert.equal(requests.length, 1);
  assert.equal(rendered.length, 0);
});

test("card fields are rendered before an explicitly submitted payment", async () => {
  const { elements, requests, rendered } = createLab();
  await elements.get("#lab-load").listeners.click();
  assert.equal(elements.get("#lab-fields").hidden, false);
  assert.deepEqual(rendered.map((item) => item.name), ["name", "number", "expiry", "cvv"]);
  assert.equal(requests.length, 1);
  await elements.get("#lab-submit").listeners.click();
  assert.equal(requests.length, 3);
  assert.equal(requests[1].url, "/api/checkout/card/orders");
  assert.deepEqual(JSON.parse(requests[1].options.body), { scenario: "success" });
  assert.match(elements.get("#lab-result").textContent, /capture completed/);
});

test("3D Secure selection is submitted with the card order", async () => {
  const { elements, requests } = createLab();
  await elements.get("#lab-load").listeners.click();
  elements.get("#lab-scenario").value = "three_ds";
  await elements.get("#lab-scenario").listeners.change();
  assert.match(elements.get("#lab-guidance").textContent, /3D Secure/);
  await elements.get("#lab-submit").listeners.click();
  assert.deepEqual(JSON.parse(requests[1].options.body), { scenario: "three_ds" });
});

test("decline or submission error does not request capture", async () => {
  const { elements, requests } = createLab({ submissionFails: true });
  await elements.get("#lab-load").listeners.click();
  elements.get("#lab-scenario").value = "decline";
  await elements.get("#lab-scenario").listeners.change();
  assert.match(elements.get("#lab-guidance").textContent, /CCREJECT-REFUSED/);
  await elements.get("#lab-submit").listeners.click();
  assert.equal(requests.length, 1);
  assert.match(elements.get("#lab-result").textContent, /not confirmed/);
  assert.equal(elements.get("#lab-submit").disabled, false);
});

test("pending capture is not displayed as a completed payment", async () => {
  const { elements } = createLab({ captureStatus: "PENDING" });
  await elements.get("#lab-load").listeners.click();
  await elements.get("#lab-submit").listeners.click();
  assert.match(elements.get("#lab-result").textContent, /PENDING/);
  assert.match(elements.get("#lab-message").textContent, /not confirmed as completed/);
});
