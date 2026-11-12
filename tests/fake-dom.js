"use strict";

// A small fake DOM for running the browser scripts in Node's vm, with a scripted fetch.
// Elements are created on first lookup by ID, so tests can preset values and inspect output.

const vm = require("node:vm");

const flush = async () => {
  for (let index = 0; index < 5; index += 1) await new Promise((resolve) => setImmediate(resolve));
};

function createElement(document, tag = "div") {
  const classes = new Set();
  const element = {
    tag, className: "", textContent: "", value: "", placeholder: "", href: "", target: "", rel: "", type: "",
    id: "", htmlFor: "", min: "", max: "", step: "", checked: false,
    hidden: false, disabled: false, children: [], listeners: {}, attributes: {}, dataset: {}, style: {},
    classList: {
      add: (...names) => names.forEach((name) => classes.add(name)),
      remove: (...names) => names.forEach((name) => classes.delete(name)),
      contains: (name) => classes.has(name),
    },
    addEventListener(event, callback) { this.listeners[event] = callback; },
    append(...children) { this.children.push(...children.map((child) => (typeof child === "string" ? { textContent: child, children: [] } : child))); },
    replaceChildren(...children) { this.children = []; this.append(...children); },
    setAttribute(name, value) { this.attributes[name] = String(value); },
    remove() { this.removed = true; },
    reset() { this.wasReset = true; },
    scrollIntoView() {}, focus() { this.focused = true; },
    querySelector(selector) {
      if (selector.startsWith("#")) return document.querySelector(selector);
      return (this.button ||= createElement(document, "button"));
    },
    querySelectorAll() { return [this.querySelector("button")]; },
    get childNodes() { return this.children; },
    get lastChild() { return (this.last ||= { textContent: "" }); },
  };
  return element;
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

function response(status, body) {
  return { ok: status >= 200 && status < 300, status, json: async () => body };
}

/** Runs `source` against a fake page. `routes` maps "METHOD url" (or a bare URL for GET) to handlers. */
async function createPage(source, routes, extraGlobals = {}) {
  const elements = new Map();
  const calls = [];
  const events = [];
  const listeners = {};
  const document = {
    querySelector(selector) {
      if (!elements.has(selector)) elements.set(selector, createElement(document));
      return elements.get(selector);
    },
    createElement: (tag) => createElement(document, tag),
    addEventListener(name, callback) { (listeners[name] ||= []).push(callback); },
    dispatchEvent(event) {
      events.push(event);
      for (const callback of listeners[event.type] || []) callback(event);
      return true;
    },
  };
  class CustomEvent {
    constructor(type, options = {}) { this.type = type; this.detail = options.detail; }
  }
  async function fetch(url, options = {}) {
    const method = options.method || "GET";
    calls.push({ method, url, body: options.body ? JSON.parse(options.body) : undefined });
    const handler = routes[`${method} ${url}`] || (method === "GET" ? routes[url] : undefined);
    if (!handler) throw new Error(`Unexpected ${method} ${url}`);
    return handler(options.body ? JSON.parse(options.body) : undefined);
  }
  const window = { location: { hash: "" } };
  vm.runInNewContext(source, {
    document, fetch, window, CustomEvent, URLSearchParams, Intl, Date, Math, Number, String, Array,
    JSON, Error, Promise, Object, globalThis: {}, navigator: { clipboard: { writeText: async () => {} } },
    ...extraGlobals,
  });
  await flush();
  return { elements, calls, events, window, $: (selector) => document.querySelector(selector) };
}

module.exports = { all, createPage, flush, response, text };
