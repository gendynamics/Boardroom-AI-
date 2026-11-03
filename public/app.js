const approveButton = document.querySelector("#approve-button");
const runButton = document.querySelector("#run-button");
const statusBadge = document.querySelector("#experiment-status");
const approvalNote = document.querySelector("#approval-note");
const resultBox = document.querySelector("#result-box");
const auditList = document.querySelector("#audit-list");
const auditEmpty = document.querySelector("#audit-empty");
const checkoutButton = document.querySelector("#checkout-button");
const checkoutMessage = document.querySelector("#checkout-message");

let demoApproved = false;
let demoRan = false;

async function finishSandboxCheckout() {
  const orderId = new URLSearchParams(window.location.search).get("token");

  if (window.location.pathname === "/checkout/cancel") {
    checkoutMessage.textContent = "Sandbox checkout was cancelled. No capture was requested.";
    return;
  }

  if (window.location.pathname !== "/checkout/complete" || !orderId) return;

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

    checkoutMessage.textContent = `Sandbox capture status: ${result.status}. This is test money only.`;
  } catch (error) {
    checkoutMessage.textContent = `${error.message} Check the server logs and Sandbox account activity.`;
  }

  window.history.replaceState({}, "", "/");
}

checkoutButton.addEventListener("click", async () => {
  checkoutButton.disabled = true;
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
    checkoutButton.disabled = false;
  }
});

finishSandboxCheckout();

function addAuditEvent(title, detail) {
  const item = document.createElement("li");
  const heading = document.createElement("strong");
  const description = document.createElement("span");
  const timestamp = document.createElement("time");

  heading.textContent = title;
  description.textContent = detail;
  timestamp.dateTime = new Date().toISOString();
  timestamp.textContent = new Date().toLocaleString();
  item.append(heading, description, timestamp);
  auditList.prepend(item);
  auditEmpty.hidden = true;
}

approveButton.addEventListener("click", () => {
  if (demoApproved) return;

  demoApproved = true;
  approveButton.disabled = true;
  approveButton.textContent = "Demo approval simulated";
  statusBadge.textContent = "DEMO APPROVED";
  statusBadge.classList.add("approved");
  runButton.disabled = false;
  approvalNote.innerHTML = '<span class="lock-icon" aria-hidden="true">ⓘ</span><span>Demo approval recorded in this page only. No real signature was created or verified; no PayPal action is authorized.</span>';
  addAuditEvent("Demo approval simulated", "10% second-order offer · 40 demo customers · 7 days · $200 discount cap");
});

runButton.addEventListener("click", () => {
  if (!demoApproved || demoRan) return;

  demoRan = true;
  runButton.disabled = true;
  statusBadge.textContent = "MOCK COMPLETE";
  statusBadge.classList.remove("approved");
  statusBadge.classList.add("complete");

  resultBox.hidden = false;
  resultBox.innerHTML = '<strong>Illustrative outcome—not measured results.</strong> This scripted example shows how an experiment report could look. No message was sent and no purchase occurred.<div class="result-metrics"><div class="result-metric"><span>Offer group conversion</span><strong>20% (8/40)</strong></div><div class="result-metric"><span>Holdout conversion</span><strong>10% (4/40)</strong></div><div class="result-metric"><span>Sample net sales lift</span><strong>+$416</strong></div></div>';
  addAuditEvent("Mock experiment completed", "Scripted illustration: 8/40 offer conversions vs. 4/40 holdout; sample net sales lift +$416.");
});
