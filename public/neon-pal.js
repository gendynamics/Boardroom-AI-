"use strict";

// Neon Pal dock: keeps the Boardroom Copilot chat one tap away on every page. The chat itself
// (conversation, suggestions, form) is a single element owned by copilot.js. Opening the dock
// moves that element into it, and visiting the Copilot page moves it back, so there is one
// conversation and nothing is duplicated. Phones get a full-screen sheet, desktops a docked panel.
(() => {
  const HOME_PAGE = "boardroom-ai";
  const launcher = document.querySelector("#pal-launcher");
  const dock = document.querySelector("#pal-dock");
  const dockBody = document.querySelector("#pal-dock-body");
  const closeButton = document.querySelector("#pal-close");
  const chat = document.querySelector("#pal-chat");
  const home = document.querySelector("#pal-home");
  const input = document.querySelector("#ai-question");
  const root = document.documentElement;
  let onHomePage = false;
  let open = false;

  function render() {
    launcher.hidden = onHomePage || open;
    dock.hidden = !open;
    launcher.setAttribute("aria-expanded", String(open));
    if (root && root.classList) {
      if (open) root.classList.add("pal-open");
      else root.classList.remove("pal-open");
    }
  }

  function openDock() {
    if (open || onHomePage) return;
    open = true;
    dockBody.append(chat);
    render();
    if (!input.disabled) input.focus();
  }

  function closeDock({ restoreFocus = true } = {}) {
    if (!open) return;
    open = false;
    render();
    if (restoreFocus && !launcher.hidden) launcher.focus();
  }

  launcher.addEventListener("click", openDock);
  closeButton.addEventListener("click", () => closeDock());

  // navigation.js announces every page change. On the Copilot page the chat lives in the page.
  document.addEventListener("boardroom:page", (event) => {
    onHomePage = Boolean(event.detail && event.detail.id === HOME_PAGE);
    if (onHomePage) {
      closeDock({ restoreFocus: false });
      home.append(chat);
    }
    render();
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && open) closeDock();
  });

  render();
})();
