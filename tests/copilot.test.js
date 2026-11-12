"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../public/copilot.js"), "utf8");
const flush = () => new Promise((resolve) => setImmediate(resolve));

function element(tag = "div") {
  return {
    tag, className: "", textContent: "", value: "", placeholder: "", href: "", target: "", rel: "", type: "",
    hidden: false, disabled: false, children: [], listeners: {}, attributes: {}, dataset: {},
    addEventListener(event, callback) { this.listeners[event] = callback; },
    append(...children) { this.children.push(...children); },
    replaceChildren(...children) { this.children = children; },
    setAttribute(name, value) { this.attributes[name] = value; },
    scrollIntoView() {}, focus() {},
    querySelector() { return (this.button ||= element("button")); },
  };
}

function all(root, predicate, found = []) {
  for (const child of root.children || []) {
    if (predicate(child)) found.push(child);
    all(child, predicate, found);
  }
  return found;
}

function text(root) {
  return [root.textContent, ...(root.children || []).map(text)].join(" ");
}

const STATUS_OK = {
  ai: { configured: true, label: "xAI Grok · grok-4.7" },
  paypal: { connected: true, orders: true, invoicing: false, transactionSearch: false },
};

const PROPOSAL = {
  id: "act-0123456789ab",
  kind: "payment_link",
  status: "proposed",
  title: "PayPal checkout link",
  fields: [{ label: "For", value: "Everyday Tote" }],
  items: [],
  total: "$45.00 USD",
  approveLabel: "Create checkout link",
  result: null,
  error: null,
};

function response(status, body) {
  return { ok: status >= 200 && status < 300, status, json: async () => body };
}

async function createPage(routes) {
  const elements = new Map();
  const calls = [];
  const document = {
    querySelector(selector) {
      if (!elements.has(selector)) elements.set(selector, element());
      return elements.get(selector);
    },
    createElement: element,
  };
  async function fetch(url, options = {}) {
    calls.push({ url, options });
    const handler = routes[url];
    if (!handler) throw new Error(`Unexpected request to ${url}`);
    return handler(options.body ? JSON.parse(options.body) : undefined);
  }
  vm.runInNewContext(source, { document, fetch, JSON, String, Array, Error });
  await flush();
  return { elements, calls };
}

async function ask(elements, question) {
  elements.get("#ai-question").value = question;
  await elements.get("#ai-form").listeners.submit({ preventDefault() {} });
}

test("status lists missing PayPal permissions and disables chat when AI is off", async () => {
  const { elements } = await createPage({
    "/api/copilot/status": () => response(200, {
      ai: { configured: false },
      paypal: { connected: true, orders: true, invoicing: false, transactionSearch: true },
    }),
  });
  const status = text(elements.get("#copilot-status"));
  assert.match(status, /AI is off/);
  assert.match(status, /Invoicing not enabled/);
  assert.match(status, /Transaction Search/);
  assert.match(status, /Apps & Credentials/);
  assert.equal(elements.get("#ai-question").disabled, true);
  assert.equal(elements.get("#copilot-badge").textContent, "AI NOT CONFIGURED");
});

test("a proposal is rendered for review and only approval asks the server to act", async () => {
  const { elements, calls } = await createPage({
    "/api/copilot/status": () => response(200, STATUS_OK),
    "/api/copilot/chat": () => response(200, {
      reply: "Your checkout link is ready for your review.",
      steps: [{ tool: "propose_payment_link", label: "Drafted checkout link · $45.00", ok: true }],
      actions: [PROPOSAL],
    }),
    "/api/copilot/actions/act-0123456789ab/approve": () => response(200, {
      action: { ...PROPOSAL, status: "done", result: { orderId: "MOCKORDER0001", approvalUrl: "https://www.sandbox.paypal.com/checkoutnow?token=MOCKORDER0001" } },
    }),
  });
  assert.equal(elements.get("#copilot-badge").textContent, "LIVE AI · SANDBOX");
  await ask(elements, "Create a $45 payment link for a tote");

  const chat = calls.find((call) => call.url === "/api/copilot/chat");
  assert.deepEqual(JSON.parse(chat.options.body), {
    messages: [{ role: "user", content: "Create a $45 payment link for a tote" }],
  });
  const conversation = elements.get("#ai-conversation");
  assert.match(text(conversation), /Drafted checkout link/);
  assert.match(text(conversation), /ready for your review/);
  assert.match(text(conversation), /Nothing has been sent to PayPal yet/);
  assert.equal(calls.filter((call) => call.url.includes("/actions/")).length, 0);

  const [approve] = all(conversation, (node) => node.textContent === "Create checkout link");
  await approve.listeners.click();
  assert.equal(calls.at(-1).url, "/api/copilot/actions/act-0123456789ab/approve");
  const [link] = all(conversation, (node) => node.tag === "a");
  assert.equal(link.href, "https://www.sandbox.paypal.com/checkoutnow?token=MOCKORDER0001");
  assert.equal(link.rel, "noopener noreferrer");
  assert.match(text(conversation), /Done/);
});

test("follow-up questions include the earlier conversation", async () => {
  const bodies = [];
  const { elements } = await createPage({
    "/api/copilot/status": () => response(200, STATUS_OK),
    "/api/copilot/chat": (body) => {
      bodies.push(body);
      return response(200, { reply: `Reply ${bodies.length}`, steps: [], actions: [] });
    },
  });
  await ask(elements, "First question");
  await ask(elements, "Second question");
  assert.deepEqual(bodies[1].messages, [
    { role: "user", content: "First question" },
    { role: "assistant", content: "Reply 1" },
    { role: "user", content: "Second question" },
  ]);
});

test("AI errors are shown without a scripted fallback and are not kept in history", async () => {
  const bodies = [];
  let fail = true;
  const { elements } = await createPage({
    "/api/copilot/status": () => response(200, STATUS_OK),
    "/api/copilot/chat": (body) => {
      bodies.push(body);
      return fail
        ? response(502, { error: "The AI provider rejected the API key.", steps: [] })
        : response(200, { reply: "OK", steps: [], actions: [] });
    },
  });
  await ask(elements, "Hello");
  assert.match(text(elements.get("#ai-conversation")), /rejected the API key\. No scripted answer was substituted/);
  fail = false;
  await ask(elements, "Hello again");
  assert.deepEqual(bodies[1].messages, [{ role: "user", content: "Hello again" }]);
});

test("links that are not PayPal are never rendered", async () => {
  const { elements } = await createPage({
    "/api/copilot/status": () => response(200, STATUS_OK),
    "/api/copilot/chat": () => response(200, { reply: "Ready.", steps: [], actions: [PROPOSAL] }),
    "/api/copilot/actions/act-0123456789ab/approve": () => response(200, {
      action: { ...PROPOSAL, status: "done", result: { orderId: "X", approvalUrl: "https://evil.example.com/pay" } },
    }),
  });
  await ask(elements, "Link please");
  const conversation = elements.get("#ai-conversation");
  const [approve] = all(conversation, (node) => node.textContent === "Create checkout link");
  await approve.listeners.click();
  assert.equal(all(conversation, (node) => node.tag === "a").length, 0);
});

test("dismissing a proposal shows that nothing was sent", async () => {
  const { elements, calls } = await createPage({
    "/api/copilot/status": () => response(200, STATUS_OK),
    "/api/copilot/chat": () => response(200, { reply: "Ready.", steps: [], actions: [PROPOSAL] }),
    "/api/copilot/actions/act-0123456789ab/dismiss": () => response(200, { action: { ...PROPOSAL, status: "dismissed" } }),
  });
  await ask(elements, "Link please");
  const conversation = elements.get("#ai-conversation");
  const [dismiss] = all(conversation, (node) => node.textContent === "Dismiss");
  await dismiss.listeners.click();
  assert.equal(calls.at(-1).url, "/api/copilot/actions/act-0123456789ab/dismiss");
  assert.match(text(conversation), /Dismissed\. Nothing was sent to PayPal/);
});

test("a collections run sends only the invoices left ticked", async () => {
  const run = {
    ...PROPOSAL,
    kind: "collections_run",
    title: "Collections run · 2 overdue invoices",
    fields: [],
    total: "$850.00 USD",
    approveLabel: "Send reminders",
    collections: {
      type: "collections_run",
      proposal_id: "act-0123456789ab",
      totals: { count: 2, amount_due: "850.00", currency: "USD" },
      items: [
        { invoice_id: "INV2-LARGE", recipient: "buyer1@example.com", amount_due: "600.00", days_overdue: 21, reason: "Largest balance, three weeks late", subject: "Reminder: invoice 0042 is past due", note: "Hi Dana, a quick nudge on invoice 0042.", selected: true },
        { invoice_id: "INV2-SMALL", recipient: "buyer2@example.com", amount_due: "250.00", days_overdue: 1, reason: "Just past due", subject: "A quick nudge", note: "Hi Sam, a friendly nudge.", selected: true },
      ],
    },
  };
  const done = {
    ...run,
    status: "done",
    result: { reminded: 1, requested: 1, results: [{ invoice_id: "INV2-LARGE", outcome: "reminded", message: null }] },
  };
  done.collections = { ...run.collections, items: [run.collections.items[0], { ...run.collections.items[1], selected: false }] };
  const { elements, calls } = await createPage({
    "/api/copilot/status": () => response(200, STATUS_OK),
    "/api/copilot/chat": () => response(200, { reply: "Ready.", steps: [], actions: [run] }),
    "/api/copilot/actions/act-0123456789ab/approve": () => response(200, { action: done }),
  });
  await ask(elements, "Chase my overdue invoices");
  const conversation = elements.get("#ai-conversation");
  assert.match(text(conversation), /21 days overdue · Largest balance/);
  assert.match(text(conversation), /1 day overdue/);
  assert.match(text(conversation), /Hi Dana, a quick nudge on invoice 0042\./);
  assert.match(text(conversation), /\$850\.00 USD/);
  const [approve] = all(conversation, (node) => node.textContent === "Send 2 reminders");
  assert.ok(approve);

  const boxes = all(conversation, (node) => node.type === "checkbox");
  assert.equal(boxes.length, 2);
  boxes[1].checked = false;
  boxes[1].listeners.change();
  assert.equal(approve.textContent, "Send 1 reminder");
  assert.match(text(conversation), /\$600\.00 USD/);
  boxes[0].checked = false;
  boxes[0].listeners.change();
  assert.equal(approve.disabled, true);
  boxes[0].checked = true;
  boxes[0].listeners.change();
  assert.equal(approve.disabled, false);

  await approve.listeners.click();
  assert.equal(calls.at(-1).url, "/api/copilot/actions/act-0123456789ab/approve");
  assert.deepEqual(JSON.parse(calls.at(-1).options.body), { invoice_ids: ["INV2-LARGE"] });
  assert.match(text(conversation), /Reminder sent/);
  assert.match(text(conversation), /PayPal accepted 1 reminder\./);
  assert.equal(all(conversation, (node) => node.type === "checkbox").every((box) => box.disabled), true);
});

test("an approved invoice draft can then be sent through the existing send endpoint", async () => {
  const invoice = {
    ...PROPOSAL,
    kind: "invoice",
    title: "Draft invoice for jordan@example.com",
    items: [{ name: "Consulting", detail: "3 hr × $60.00/hr", amount: "$180.00" }],
    total: "$180.00 USD",
    approveLabel: "Create Sandbox draft invoice",
  };
  const { elements, calls } = await createPage({
    "/api/copilot/status": () => response(200, STATUS_OK),
    "/api/copilot/chat": () => response(200, { reply: "Ready.", steps: [], actions: [invoice] }),
    "/api/copilot/actions/act-0123456789ab/approve": () => response(200, { action: { ...invoice, status: "done", result: { invoiceId: "INV2-AAAA-BBBB" } } }),
    "/api/sandbox/invoices/INV2-AAAA-BBBB/send": () => response(200, { status: "sent" }),
  });
  await ask(elements, "Invoice Jordan");
  const conversation = elements.get("#ai-conversation");
  assert.match(text(conversation), /3 hr × \$60\.00\/hr/);
  const [approve] = all(conversation, (node) => node.textContent === "Create Sandbox draft invoice");
  await approve.listeners.click();
  const [send] = all(conversation, (node) => node.textContent === "Send invoice");
  await send.listeners.click();
  assert.equal(calls.at(-1).url, "/api/sandbox/invoices/INV2-AAAA-BBBB/send");
  assert.match(text(conversation), /PayPal accepted the send request for INV2-AAAA-BBBB/);
});
