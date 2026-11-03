# The Boardroom — PayPal Hackathon Demo

An independent hackathon prototype for turning merchant transaction patterns into a practical, measurable growth experiment.

> **Demo only:** The dashboard uses synthetic transaction data and scripted experiment outcomes. The separate checkout test connects only to PayPal Sandbox and uses fake money. The experiment approval button simulates a UI state only; it does not create or verify an ML-DSA signature or authorize any real action.

## Run locally

1. Install Rust and Cargo.
2. Copy `.env.example` to `.env` and add your PayPal **Sandbox** Client ID and Secret from the Developer Dashboard.
3. Run `cargo run`.
4. Open `http://localhost:3000`.

The backend is written in Rust and serves the dashboard and PayPal API routes. It calls PayPal's Sandbox REST API directly; it does not use a Node.js server or the deprecated Checkout Node SDK. The dashboard still uses synthetic business analytics. The optional checkout card creates a fixed $10 USD Sandbox order, redirects the buyer to PayPal Sandbox for approval, and requests capture after PayPal returns to the app. It uses fake Sandbox money only. The server requires credentials and is deliberately restricted to Sandbox; it does not support live payments.

The `APP_BASE_URL` setting defaults to `http://localhost:3000`; set it to the app's public origin when testing a hosted Sandbox checkout. The demo keeps created order IDs in memory so the capture endpoint only accepts an order created by this running process. Restarting the server clears this temporary state. Do not deploy this demo as-is.

## Negative testing

Negative testing is disabled by default. To try the configured Sandbox error simulation, set `ENABLE_NEGATIVE_TESTING=true` in `.env` and choose an allowed `NEGATIVE_TEST_TYPE`. PayPal may support different mock error codes for different API operations. Turn the setting off after testing.

## Demo flow

1. Review the synthetic 30-day business snapshot and four advisor perspectives.
2. Read the proposed 7-day, 10% second-order offer and its audience and discount cap.
3. Select **Simulate approval** to record a demo-only approval event.
4. Select **Run mock experiment** to reveal a clearly labeled illustrative result and add it to the on-page demo audit trail.
5. Optionally test the separate fixed-value $10 Sandbox checkout flow.

The experiment proposal does not send offers to customers. Its data and result are fixed illustrations, not merchant performance or measured experiment findings. The separate checkout flow creates a Sandbox order only. No anonymized network benchmark is included.

## Before connecting real services

- Replace all sample data with an authenticated, least-privilege PayPal integration and show the source and period for every metric.
- Implement server-side verification of a cryptographic approval bound to the exact action details. The front-end demo control is not a security boundary.
- Keep secrets on the server; do not put credentials in browser code or commit them to the repository.
- Add durable, access-controlled, tamper-evident audit storage and record outcomes from the actual integration.
- Measure incremental results against a suitable holdout, accounting for refunds, discount costs, and contribution margin.

This repository is an independent hackathon submission and is not affiliated with or endorsed by PayPal, Inc. PayPal is a trademark of PayPal, Inc.
