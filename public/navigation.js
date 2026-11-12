"use strict";

(() => {
  const categories = [
    { id: "home", label: "Home", pages: [
      { id: "home", label: "Home", selectors: ["#home"] },
    ] },
    { id: "dashboard", label: "Dashboard", pages: [
      { id: "business-overview", label: "Overview", selectors: ["#business-overview", ".metrics-grid", ".insight-strip", ".dashboard-feature"] },
      { id: "sales-snapshot", label: "Sales snapshot", selectors: ["#sales-snapshot"] },
    ] },
    { id: "activity", label: "Activity", pages: [
      { id: "activity", label: "Transactions", selectors: ["#activity"] },
      { id: "demo-audit", label: "Audit trail", selectors: ["#demo-audit", ".benchmark-note"] },
    ] },
    { id: "payments", label: "Send & request", pages: [
      { id: "send-request", label: "Send & request", selectors: ["#send-request"] },
      { id: "checkout-lab", label: "Checkout Lab", selectors: ["#checkout-lab"] },
      { id: "sandbox-purchase", label: "Sandbox purchase", selectors: ["#sandbox-purchase"] },
    ] },
    { id: "wallet", label: "Wallet", pages: [
      { id: "wallet", label: "Payment methods", selectors: ["#wallet"] },
    ] },
    { id: "invoicing", label: "Invoicing", pages: [
      { id: "invoicing", label: "Invoices", selectors: ["#invoicing"] },
    ] },
    { id: "boardroom", label: "Boardroom AI", pages: [
      { id: "boardroom-ai", label: "Copilot", selectors: ["#boardroom-ai"] },
      { id: "boardroom", label: "The vision", selectors: ["#boardroom"] },
      { id: "boardroom-recommendation", label: "Recommendation", selectors: ["#boardroom-recommendation"] },
      { id: "experiment", label: "Experiment", selectors: ["#experiment"] },
    ] },
    { id: "settings", label: "Settings", pages: [
      { id: "settings", label: "Profile", selectors: ["#settings"] },
    ] },
  ];

  const pages = categories.flatMap((category) => category.pages.map((page) => ({ ...page, category })));
  const main = document.querySelector("main");
  const firstSection = document.querySelector("#home");
  const navigation = document.querySelector("#page-navigation");
  const subcategories = document.querySelector("#subcategory-nav");
  const position = document.querySelector("#page-position");
  const previous = document.querySelector("#previous-page");
  const next = document.querySelector("#next-page");
  const container = document.createElement("div");
  container.className = "page-viewport";
  main.insertBefore(container, firstSection);

  for (const page of pages) {
    page.element = document.createElement("div");
    page.element.className = "page-view";
    page.element.hidden = true;
    page.element.tabIndex = -1;
    page.element.setAttribute("aria-label", `${page.category.label}: ${page.label}`);
    for (const selector of page.selectors) {
      const element = document.querySelector(selector);
      if (!element) throw new Error(`Missing workspace section: ${selector}`);
      page.element.append(element);
    }
    container.append(page.element);
  }
  document.querySelectorAll(".workspace-grid, .content-grid").forEach((wrapper) => wrapper.remove());
  navigation.hidden = false;

  let current = -1;
  function pageForHash(hash) {
    const id = hash.replace(/^#/, "");
    if (!id) return 0;
    const target = document.getElementById(id);
    if (!target) return -1;
    return pages.findIndex((page) => page.element.contains(target));
  }

  function show(index, { focus = false } = {}) {
    if (index < 0 || index >= pages.length) return;
    const direction = index < current ? "slide-from-left" : "slide-from-right";
    if (current >= 0) pages[current].element.hidden = true;
    current = index;
    const page = pages[index];
    page.element.classList.remove("slide-from-left", "slide-from-right");
    page.element.classList.add(direction);
    page.element.hidden = false;
    document.querySelectorAll(".business-nav a[data-category]").forEach((link) => {
      if (link.dataset.category === page.category.id) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    });
    subcategories.replaceChildren();
    for (const item of page.category.pages) {
      const link = document.createElement("a");
      link.href = `#${item.id}`;
      link.textContent = item.label;
      if (item.id === page.id) link.setAttribute("aria-current", "page");
      subcategories.append(link);
    }
    position.textContent = `${page.category.label} / ${page.label}`;
    previous.disabled = index === 0;
    next.disabled = index === pages.length - 1;
    if (focus) {
      window.scrollTo({ top: 0, behavior: "instant" });
      page.element.focus({ preventScroll: true });
    }
  }

  function go(index) {
    if (index < 0 || index >= pages.length) return;
    history.pushState({}, "", `#${pages[index].id}`);
    show(index, { focus: true });
  }

  document.addEventListener("click", (event) => {
    const link = event.target.closest("a[href^='#']");
    if (!link || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const index = pageForHash(link.getAttribute("href"));
    if (index < 0) return;
    event.preventDefault();
    go(index);
  });
  previous.addEventListener("click", () => go(current - 1));
  next.addEventListener("click", () => go(current + 1));
  window.addEventListener("hashchange", () => {
    const index = pageForHash(window.location.hash);
    if (index >= 0) show(index, { focus: true });
  });

  let touchStart = null;
  container.addEventListener("touchstart", (event) => {
    if (event.touches.length !== 1 || event.target.closest("input, select, textarea, button, a")) return;
    touchStart = { x: event.touches[0].clientX, y: event.touches[0].clientY };
  }, { passive: true });
  container.addEventListener("touchend", (event) => {
    if (!touchStart || event.changedTouches.length !== 1) return;
    const dx = event.changedTouches[0].clientX - touchStart.x;
    const dy = event.changedTouches[0].clientY - touchStart.y;
    touchStart = null;
    if (Math.abs(dx) >= 70 && Math.abs(dx) > Math.abs(dy) * 1.5) go(current + (dx < 0 ? 1 : -1));
  }, { passive: true });
  container.addEventListener("touchcancel", () => { touchStart = null; }, { passive: true });

  const initial = window.location.pathname.startsWith("/checkout/")
    ? pageForHash("#sandbox-purchase")
    : pageForHash(window.location.hash);
  show(initial >= 0 ? initial : 0);
})();
