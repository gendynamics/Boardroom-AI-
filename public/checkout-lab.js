"use strict";

const labScenario = document.querySelector("#lab-scenario");
const labGuidance = document.querySelector("#lab-guidance");
const labLoad = document.querySelector("#lab-load");
const labFields = document.querySelector("#lab-fields");
const labSubmit = document.querySelector("#lab-submit");
const labMessage = document.querySelector("#lab-message");
const labPrepared = document.querySelector("#lab-prepared");
const labApproval = document.querySelector("#lab-approval");
const labResult = document.querySelector("#lab-result");

const scenarioGuidance = {
  success: "Use a Sandbox test card, a future expiration date, and a test security code from PayPal’s guide.",
  decline: "Use a Sandbox test card and enter CCREJECT-REFUSED as the name on the card. PayPal’s rejection triggers are case-sensitive.",
  three_ds: "Use a purchase-flow 3D Secure test card from PayPal’s guide. A challenge is not guaranteed for every card or region.",
};

let labCardFields;
let labSubmitting = false;
let labApproved = false;

function labSetBusy(busy) {
  labSubmitting = busy;
  labSubmit.disabled = busy;
  labScenario.disabled = busy;
}

async function labJson(response) {
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || "Sandbox request failed. Check the server logs.");
  return result;
}

function labLoadSdk(clientId) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    const params = new URLSearchParams({
      "client-id": clientId,
      components: "card-fields",
      currency: "USD",
      intent: "capture",
    });
    script.src = `https://www.paypal.com/sdk/js?${params}`;
    script.onload = resolve;
    script.onerror = () => reject(new Error("Could not load PayPal’s Sandbox card fields."));
    document.head.appendChild(script);
  });
}

labScenario.addEventListener("change", () => {
  labGuidance.textContent = scenarioGuidance[labScenario.value];
});

labLoad.addEventListener("click", async () => {
  labLoad.disabled = true;
  labMessage.textContent = "Checking Sandbox card-field availability…";
  try {
    const config = await labJson(await fetch("/api/checkout/card/config"));
    await labLoadSdk(config.clientId);
    if (!window.paypal || typeof window.paypal.CardFields !== "function") {
      throw new Error("PayPal’s card fields are not available in this browser.");
    }

    labCardFields = window.paypal.CardFields({
      createOrder: async () => {
        labPrepared.textContent = "Prepared: creating a $10.00 USD Sandbox order";
        const order = await labJson(await fetch("/api/checkout/card/orders", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ scenario: labScenario.value }),
        }));
        labPrepared.textContent = "Prepared: Sandbox order created";
        labApproval.textContent = "Card verification: awaiting PayPal";
        return order.orderId;
      },
      onApprove: async (data) => {
        labApproved = true;
        labApproval.textContent = "Card verification: PayPal approved the order";
        labResult.textContent = "Payment result: requesting capture";
        try {
          const result = await labJson(await fetch(`/api/checkout/orders/${encodeURIComponent(data.orderID)}/capture`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: "{}",
          }));
          if (result.status === "COMPLETED") {
            labResult.textContent = "Payment result: capture completed in Sandbox";
            labMessage.textContent = "Sandbox payment completed using test money.";
          } else {
            labResult.textContent = `Payment result: ${result.status}; verify Sandbox activity`;
            labMessage.textContent = "Capture is not confirmed as completed. Check Sandbox activity before retrying.";
          }
        } catch (error) {
          labResult.textContent = "Payment result: capture could not be confirmed";
          labMessage.textContent = `${error.message} Check Sandbox activity before retrying.`;
        }
      },
      onError: (error) => {
        console.error("PayPal Sandbox card checkout failed:", error);
        labApproval.textContent = "Card verification: not completed";
        labResult.textContent = "Payment result: not confirmed";
        labMessage.textContent = "PayPal could not complete this card payment. Check the test details and Sandbox activity before trying again.";
      },
    });

    if (!labCardFields.isEligible()) {
      labMessage.textContent = "Card fields are unavailable for this Sandbox account. Check Advanced Credit and Debit Card Payments eligibility in PayPal’s developer dashboard.";
      return;
    }

    await Promise.all([
      labCardFields.NameField().render("#lab-card-name"),
      labCardFields.NumberField().render("#lab-card-number"),
      labCardFields.ExpiryField().render("#lab-card-expiry"),
      labCardFields.CVVField().render("#lab-card-cvv"),
    ]);
    labFields.hidden = false;
    labLoad.hidden = true;
    labMessage.textContent = "Review the scenario and enter only PayPal Sandbox test card details.";
  } catch (error) {
    console.error("PayPal Sandbox card fields could not be initialized:", error);
    labMessage.textContent = `${error.message} Check the server logs and Sandbox configuration.`;
    if (window.paypal) {
      labMessage.textContent += " Reload the page before trying again.";
    } else {
      labLoad.disabled = false;
    }
  }
});

labSubmit.addEventListener("click", async () => {
  if (labSubmitting || !labCardFields) return;
  labSetBusy(true);
  labApproved = false;
  labPrepared.textContent = "Prepared: submitting the test purchase";
  labApproval.textContent = "Card verification: awaiting PayPal";
  labResult.textContent = "Payment result: not yet available";
  labMessage.textContent = "Submitting to PayPal Sandbox. Complete any verification PayPal requests.";
  try {
    await labCardFields.submit();
    if (!labApproved) {
      labResult.textContent = "Payment result: not confirmed";
      labMessage.textContent = "PayPal did not confirm approval. Check Sandbox activity before trying again.";
    }
  } catch (error) {
    console.error("PayPal Sandbox card submission failed:", error);
    labApproval.textContent = "Card verification: not completed";
    labResult.textContent = "Payment result: not confirmed";
    labMessage.textContent = "The test payment was not confirmed. Check the card details and Sandbox activity before trying again.";
  } finally {
    labSetBusy(false);
  }
});
