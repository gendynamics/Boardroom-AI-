// Fixed $10 Sandbox checkout: review, create the order, then capture when PayPal returns the buyer.
const checkoutButton = document.querySelector("#checkout-button");
const checkoutMessage = document.querySelector("#checkout-message");
const purchaseReview = document.querySelector("#purchase-review");
const purchaseReviewHeading = document.querySelector("#purchase-review-heading");
const checkoutConfirm = document.querySelector("#checkout-confirm");
const checkoutCancelReview = document.querySelector("#checkout-cancel-review");

async function finishSandboxCheckout() {
  const orderId = new URLSearchParams(window.location.search).get("token");

  if (window.location.pathname === "/checkout/cancel") {
    checkoutMessage.textContent = "Sandbox checkout was cancelled. No capture was requested.";
    window.history.replaceState({}, "", "/#sandbox-purchase");
    return;
  }

  if (window.location.pathname !== "/checkout/complete") return;
  if (!orderId) {
    checkoutMessage.textContent = "PayPal did not return an order ID. Check Sandbox account activity before trying again.";
    window.history.replaceState({}, "", "/#sandbox-purchase");
    return;
  }

  checkoutButton.disabled = true;
  checkoutMessage.textContent = "Buyer approval returned. Confirming Sandbox capture…";

  try {
    const response = await fetch(`/api/checkout/orders/${encodeURIComponent(orderId)}/capture`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });
    const result = await response.json();

    if (!response.ok) throw new Error(result.error || "Sandbox capture failed.");

    checkoutMessage.textContent = result.status === "COMPLETED"
      ? "Sandbox capture completed. It now appears on your dashboard and in Activity. This is test money only."
      : `Sandbox capture status: ${result.status}. Check Sandbox activity before trying again.`;
    if (typeof CustomEvent === "function" && typeof document.dispatchEvent === "function") {
      document.dispatchEvent(new CustomEvent("boardroom:changed"));
    }
  } catch (error) {
    checkoutMessage.textContent = `${error.message} Check the server logs and Sandbox account activity.`;
  }

  checkoutButton.disabled = false;
  window.history.replaceState({}, "", "/#sandbox-purchase");
}

checkoutButton.addEventListener("click", () => {
  checkoutButton.hidden = true;
  purchaseReview.hidden = false;
  checkoutMessage.textContent = "Review the order before continuing to PayPal Sandbox.";
  purchaseReviewHeading.focus();
});

checkoutCancelReview.addEventListener("click", () => {
  purchaseReview.hidden = true;
  checkoutButton.hidden = false;
  checkoutMessage.textContent = "Purchase review cancelled. No Sandbox order was created.";
  checkoutButton.focus();
});

checkoutConfirm.addEventListener("click", async () => {
  checkoutConfirm.disabled = true;
  checkoutCancelReview.disabled = true;
  checkoutMessage.textContent = "Creating a Sandbox order…";

  try {
    const response = await fetch("/api/checkout/orders", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });
    const result = await response.json();

    if (!response.ok) throw new Error(result.error || "Could not create a Sandbox order.");

    checkoutMessage.textContent = "Sandbox order created. Redirecting to PayPal for buyer approval…";
    window.location.assign(result.approvalUrl);
  } catch (error) {
    checkoutMessage.textContent = error.message;
    checkoutConfirm.disabled = false;
    checkoutCancelReview.disabled = false;
  }
});

finishSandboxCheckout();
