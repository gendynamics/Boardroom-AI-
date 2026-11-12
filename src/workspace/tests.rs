use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};

use axum::{
    Router,
    body::to_bytes,
    http::HeaderValue,
    routing::{get, post},
};
use tokio::net::TcpListener;

use super::*;
use crate::ledger::{Data, OrderRecord, RefundRecord};

// ---------------------------------------------------------------------------------------------
// Dashboard figures

const NOW: i64 = 1_800_000_000;

fn paid(id: &str, days_ago: f64, cents: i64, payer: Option<&str>, item: &str) -> OrderRecord {
    let at = NOW - (days_ago * DAY as f64) as i64;
    OrderRecord {
        id: id.to_owned(),
        created_at: at - 60,
        source: "request".to_owned(),
        description: item.to_owned(),
        amount_cents: cents,
        currency: "USD".to_owned(),
        state: "captured".to_owned(),
        verification: "paypal".to_owned(),
        customer: None,
        approval_url: None,
        captured_at: Some(at),
        capture_id: None,
        captured_cents: cents,
        payer_email: payer.map(str::to_owned),
        payer_name: None,
        refunds: Vec::new(),
    }
}

fn sample_data() -> Data {
    let mut data = Data::default();
    // Previous 30 days: $100 from a@.
    data.orders.push(paid("PREV00000000001", 40.0, 10_000, Some("a@example.com"), "Tote"));
    // Last 30 days: $45 + $45 from a@ (repeat), $60 from b@, $30 from c@.
    data.orders.push(paid("CURR00000000001", 20.0, 4_500, Some("a@example.com"), "Tote"));
    data.orders.push(paid("CURR00000000002", 10.0, 6_000, Some("B@example.com"), "Weekend Bag"));
    data.orders.push(paid("CURR00000000003", 2.5, 4_500, Some("a@example.com"), "Tote"));
    data.orders.push(paid("CURR00000000004", 0.5, 3_000, Some("c@example.com"), "Pouch"));
    data.orders[4].refunds.push(RefundRecord {
        id: None,
        at: NOW - 3_600,
        cents: 500,
    });
    // An unpaid payment request and an unpaid card order (card orders are not "links").
    let mut unpaid = paid("UNPAID000000001", 1.0, 2_500, None, "Deposit");
    unpaid.state = "created".to_owned();
    unpaid.captured_at = None;
    unpaid.captured_cents = 0;
    data.orders.push(unpaid.clone());
    let mut card = unpaid;
    card.id = "UNPAIDCARD00001".to_owned();
    card.source = "card".to_owned();
    data.orders.push(card);
    data
}

#[test]
fn dashboard_is_computed_from_recorded_captures_and_refunds() {
    let summary = summarize(&sample_data(), NOW, 30);
    assert_eq!(summary["hasData"], true);
    assert_eq!(summary["gross"]["text"], "$180.00");
    assert_eq!(summary["gross"]["changePct"], 80.0);
    assert_eq!(summary["orders"]["count"], 4);
    assert_eq!(summary["orders"]["previous"], 1);
    assert_eq!(summary["averageOrder"]["text"], "$45.00");
    assert_eq!(summary["averageOrder"]["changePct"], -55.0);
    // a@ has 3 paid orders, b@ (case-insensitive) and c@ one each: 1 of 3 customers repeat.
    assert_eq!(summary["repeatCustomers"]["customers"], 3);
    assert_eq!(summary["repeatCustomers"]["repeat"], 1);
    assert_eq!(summary["repeatCustomers"]["pct"], 33.3);
    assert_eq!(summary["refunds"]["text"], "$5.00");
    assert_eq!(summary["refunds"]["pctOfGross"], 2.8);
    assert_eq!(summary["net"]["text"], "$175.00");
    assert_eq!(summary["unpaid"]["count"], 1);
    assert_eq!(summary["unpaid"]["text"], "$25.00");

    let series = summary["series"].as_array().unwrap();
    assert_eq!(series.len(), 30);
    let total: i64 = series.iter().map(|day| day["cents"].as_i64().unwrap()).sum();
    assert_eq!(total, 18_000);
    assert_eq!(series[29]["cents"], 3_000);
    assert_eq!(series[29]["date"], &utc_timestamp(NOW)[..10]);
    let buckets = summary["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 12);
    let bucket_total: i64 = buckets.iter().map(|b| b["gross"].as_i64().unwrap()).sum();
    assert_eq!(bucket_total, 18_000);

    assert_eq!(summary["topItems"][0]["name"], "Tote");
    assert_eq!(summary["topItems"][0]["text"], "$90.00");
    assert_eq!(summary["topItems"][0]["orders"], 2);
    assert_eq!(summary["topItems"].as_array().unwrap().len(), 3);
    assert_eq!(summary["insight"]["tag"], "GROWTH");
}

#[test]
fn short_ranges_still_fill_every_bucket_and_empty_data_says_so() {
    let summary = summarize(&sample_data(), NOW, 7);
    assert_eq!(summary["series"].as_array().unwrap().len(), 7);
    let bucket_total: i64 = summary["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["gross"].as_i64().unwrap())
        .sum();
    // Seven days over twelve buckets: each day is counted in at least one bucket.
    assert!(bucket_total >= 7_500);

    let empty = summarize(&Data::default(), NOW, 30);
    assert_eq!(empty["hasData"], false);
    assert_eq!(empty["gross"]["text"], "$0.00");
    assert!(empty["gross"]["changePct"].is_null());
    assert!(empty["averageOrder"]["cents"].is_null());
    assert!(empty["repeatCustomers"]["pct"].is_null());
    assert_eq!(empty["insight"]["tag"], "GET STARTED");
}

#[test]
fn insights_flag_high_refunds_before_anything_else() {
    let mut data = sample_data();
    data.orders[2].refunds.push(RefundRecord {
        id: None,
        at: NOW - 100,
        cents: 4_000,
    });
    let summary = summarize(&data, NOW, 30);
    assert_eq!(summary["insight"]["tag"], "REFUNDS");
    assert!(
        summary["insight"]["title"]
            .as_str()
            .unwrap()
            .contains("25.0%")
    );
}

#[test]
fn wallet_balance_counts_refunds_and_successful_payouts_only() {
    let mut data = sample_data();
    for (status, cents) in [("SUCCESS", 1_000), ("UNCLAIMED", 500), ("DENIED", 9_999)] {
        data.payouts.push(ledger::PayoutRecord {
            batch_id: format!("BATCH{status}"),
            request_id: String::new(),
            created_at: NOW,
            recipient: "x@example.com".to_owned(),
            amount_cents: cents,
            note: None,
            status: status.to_owned(),
        });
    }
    let balance = boardroom_balance(&data);
    assert_eq!(balance["captured"], "$280.00");
    assert_eq!(balance["refunded"], "$5.00");
    assert_eq!(balance["paidOut"], "$15.00");
    assert_eq!(balance["net"], "$260.00");
    assert_eq!(balance["payments"], 5);
}

#[test]
fn invoice_status_sync_keeps_local_send_states() {
    assert_eq!(synced_status("draft", "DRAFT"), None);
    assert_eq!(synced_status("sent", "SENT"), None);
    assert_eq!(synced_status("sent", "UNPAID"), None);
    assert_eq!(synced_status("sent", "PAID").as_deref(), Some("PAID"));
    assert_eq!(synced_status("PAID", "PAID"), None);
    assert_eq!(synced_status("sent", "CANCELLED").as_deref(), Some("cancelled"));
}

#[test]
fn payout_body_sends_one_usd_email_item() {
    let body = payout_body(
        "123e4567-e89b-42d3-a456-426614174000",
        "sb-buyer@personal.example.com",
        1_250,
        None,
        "Avery's Bags",
    );
    let header = &body["sender_batch_header"];
    assert_eq!(header["sender_batch_id"], "123e4567-e89b-42d3-a456-426614174000");
    assert_eq!(header["email_subject"], "You have a payment from Avery's Bags");
    let item = &body["items"][0];
    assert_eq!(item["recipient_type"], "EMAIL");
    assert_eq!(item["receiver"], "sb-buyer@personal.example.com");
    assert_eq!(item["amount"], json!({ "value": "12.50", "currency": "USD" }));
    assert_eq!(item["sender_item_id"], "item-123e4567");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------------------------
// Endpoints against a mock PayPal

const PAYOUTS: &str = "https://uri.paypal.com/payments/payouts";
const INVOICING: &str = "https://uri.paypal.com/services/invoicing";
const REQUEST_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

#[derive(Clone)]
struct Mock {
    base: String,
    scope: String,
    requests: Arc<StdMutex<Vec<(String, Option<String>, Value)>>>,
}

impl Mock {
    fn record(&self, path: String, headers: &HeaderMap, body: Value) {
        let request_id = headers
            .get("PayPal-Request-Id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.requests.lock().unwrap().push((path, request_id, body));
    }

    fn posts_to(&self, path: &str) -> Vec<(Option<String>, Value)> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(recorded, _, _)| recorded == path)
            .map(|(_, request_id, body)| (request_id.clone(), body.clone()))
            .collect()
    }
}

fn mock_order(id: &str) -> Value {
    json!({
        "id": id,
        "status": "COMPLETED",
        "payer": { "email_address": "sb-buyer@personal.example.com", "name": { "given_name": "Avery", "surname": "Chen" } },
        "purchase_units": [{
            "amount": { "currency_code": "USD", "value": "45.00" },
            "payments": { "captures": [{ "id": "CAPTUREWS000001", "status": "COMPLETED", "amount": { "currency_code": "USD", "value": "45.00" } }] }
        }]
    })
}

async fn spawn_mock(scope: &str) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock = Mock {
        base: format!("http://{}", listener.local_addr().unwrap()),
        scope: scope.to_owned(),
        requests: Arc::default(),
    };
    let router = Router::new()
        .route(
            "/v1/oauth2/token",
            post(|State(mock): State<Mock>| async move {
                Json(json!({ "access_token": "mock", "expires_in": 32_400, "scope": mock.scope }))
            }),
        )
        .route(
            "/v2/checkout/orders",
            post(|State(mock): State<Mock>, headers: HeaderMap, Json(body): Json<Value>| async move {
                mock.record("/v2/checkout/orders".to_owned(), &headers, body);
                Json(json!({
                    "id": "WSORDER00000001",
                    "status": "PAYER_ACTION_REQUIRED",
                    "links": [{ "rel": "payer-action", "href": "https://www.sandbox.paypal.com/checkoutnow?token=WSORDER00000001" }]
                }))
            }),
        )
        .route(
            "/v2/checkout/orders/{id}",
            get(|Path(id): Path<String>| async move { Json(mock_order(&id)) }),
        )
        .route(
            "/v2/checkout/orders/{id}/capture",
            // A minimal capture response, so the app must read the order back for details.
            post(|Path(id): Path<String>| async move { Json(json!({ "id": id, "status": "COMPLETED" })) }),
        )
        .route(
            "/v2/payments/captures/{id}/refund",
            post(|State(mock): State<Mock>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>| async move {
                mock.record(format!("/v2/payments/captures/{id}/refund"), &headers, body);
                Json(json!({ "id": "REFUNDWS0000001", "status": "COMPLETED" }))
            }),
        )
        .route(
            "/v1/payments/payouts",
            post(|State(mock): State<Mock>, headers: HeaderMap, Json(body): Json<Value>| async move {
                mock.record("/v1/payments/payouts".to_owned(), &headers, body);
                (StatusCode::CREATED, Json(json!({ "batch_header": { "payout_batch_id": "BATCHWS12345", "batch_status": "PENDING" } })))
            }),
        )
        .route(
            "/v1/payments/payouts/{id}",
            get(|| async { Json(json!({ "batch_header": { "batch_status": "SUCCESS" }, "items": [{ "transaction_status": "UNCLAIMED" }] })) }),
        )
        .route(
            "/v2/invoicing/invoices",
            post(|State(mock): State<Mock>, headers: HeaderMap, Json(body): Json<Value>| async move {
                mock.record("/v2/invoicing/invoices".to_owned(), &headers, body);
                (StatusCode::CREATED, Json(json!({ "href": format!("{}/v2/invoicing/invoices/INV2-WS00-0001", mock.base) })))
            })
            .get(|| async {
                Json(json!({ "total_items": 1, "items": [{
                    "id": "INV2-WS00-0001", "status": "PAID",
                    "detail": { "invoice_number": "0001", "metadata": { "recipient_view_url": "https://www.sandbox.paypal.com/invoice/p/#INV2WS000001" } },
                    "primary_recipients": [{ "billing_info": { "email_address": "jordan@example.com" } }],
                    "amount": { "currency_code": "USD", "value": "205.00" }
                }] }))
            }),
        )
        .route(
            "/v2/invoicing/invoices/{id}/cancel",
            post(|State(mock): State<Mock>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>| async move {
                mock.record(format!("/v2/invoicing/invoices/{id}/cancel"), &headers, body);
                StatusCode::NO_CONTENT
            }),
        )
        .with_state(mock.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    mock
}

fn state_for(mock: &Mock) -> AppState {
    AppState {
        client: reqwest::Client::new(),
        port: 3000,
        paypal_api: mock.base.clone(),
        client_id: "test".to_owned(),
        client_secret: "test".to_owned(),
        app_base_url: "http://localhost:3000".to_owned(),
        public_directory: PathBuf::from("public"),
        negative_testing_enabled: false,
        negative_test_type: "INSUFFICIENT_FUNDS".to_owned(),
        orders: Arc::default(),
        order_log: Arc::default(),
        invoices: Arc::default(),
        token_cache: Arc::default(),
        ai: None,
        copilot: Arc::default(),
        ledger: Arc::default(),
        brief_busy: Arc::default(),
    }
}

fn origin() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::ORIGIN, HeaderValue::from_static("http://localhost:3000"));
    headers
}

async fn body(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn a_paid_request_is_recorded_from_creation_through_capture_and_refund() {
    let mock = spawn_mock("openid").await;
    let state = state_for(&mock);

    // A cross-origin request is refused before PayPal is called.
    let request = || parse(json!({ "amount": "45", "description": "Everyday Tote", "customer": "Jordan" }));
    let (status, _) = body(request_payment(State(state.clone()), HeaderMap::new(), Json(request())).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(mock.posts_to("/v2/checkout/orders").is_empty());

    // The profile's business name becomes the checkout brand.
    let (status, _) = body(
        update_profile(
            State(state.clone()),
            origin(),
            Json(parse(json!({ "displayName": "Avery", "businessName": "Avery's Bags" }))),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, created) = body(request_payment(State(state.clone()), origin(), Json(request())).await).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["orderId"], "WSORDER00000001");
    let posted = mock.posts_to("/v2/checkout/orders");
    assert_eq!(posted[0].1["purchase_units"][0]["amount"]["value"], "45.00");
    assert_eq!(
        posted[0].1["payment_source"]["paypal"]["experience_context"]["brand_name"],
        "Avery's Bags"
    );
    {
        let ledger = state.ledger.lock().await;
        let order = &ledger.data.orders[0];
        assert_eq!((order.source.as_str(), order.state.as_str()), ("request", "created"));
        assert_eq!(order.customer.as_deref(), Some("Jordan"));
    }

    // Buyer returns from PayPal: capture reads details back and records the payer.
    let (status, captured) = body(crate::capture_order(State(state.clone()), Path("WSORDER00000001".to_owned())).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(captured["status"], "COMPLETED");
    {
        let ledger = state.ledger.lock().await;
        let order = &ledger.data.orders[0];
        assert!(order.is_paid());
        assert_eq!(order.captured_cents, 4_500);
        assert_eq!(order.capture_id.as_deref(), Some("CAPTUREWS000001"));
        assert_eq!(order.payer_name.as_deref(), Some("Avery Chen"));
    }

    // Refund over the balance is refused by PayPal's current figures; a valid one is recorded once.
    let refund = |amount: &str| parse(json!({ "amount": amount, "requestId": REQUEST_ID }));
    let (status, _) = body(refund_order(State(state.clone()), origin(), Path("WSORDER00000001".to_owned()), Json(refund("50.00"))).await).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    for _ in 0..2 {
        let (status, refunded) = body(refund_order(State(state.clone()), origin(), Path("WSORDER00000001".to_owned()), Json(refund("5"))).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(refunded["refundId"], "REFUNDWS0000001");
    }
    let refunds = mock.posts_to("/v2/payments/captures/CAPTUREWS000001/refund");
    assert_eq!(refunds[0].0.as_deref(), Some(REQUEST_ID));
    let ledger = state.ledger.lock().await;
    assert_eq!(ledger.data.orders[0].refunded_cents(), 500, "an idempotent retry is counted once");
    let titles: Vec<&str> = ledger.data.events.iter().map(|event| event.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Profile updated", "Payment request created", "Payment captured", "Refund issued"]
    );
}

#[tokio::test]
async fn payouts_need_permission_a_test_recipient_and_a_fresh_request_id() {
    let denied = spawn_mock("openid").await;
    let payout = |recipient: &str| {
        parse::<PayoutRequest>(json!({ "recipient": recipient, "amount": "12.5", "requestId": REQUEST_ID }))
    };
    let (status, reply) = body(create_payout(State(state_for(&denied)), origin(), Json(payout("sb-buyer@personal.example.com"))).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(reply["error"].as_str().unwrap().contains("Developer Dashboard"));

    let mock = spawn_mock(PAYOUTS).await;
    let state = state_for(&mock);
    let (status, _) = body(create_payout(State(state.clone()), origin(), Json(payout("someone@gmail.com"))).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(mock.posts_to("/v1/payments/payouts").is_empty());

    let (status, sent) = body(create_payout(State(state.clone()), origin(), Json(payout("SB-Buyer@personal.example.com"))).await).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(sent["batchId"], "BATCHWS12345");
    let posted = mock.posts_to("/v1/payments/payouts");
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].0.as_deref(), Some(REQUEST_ID));
    assert_eq!(posted[0].1["items"][0]["receiver"], "sb-buyer@personal.example.com");
    assert_eq!(posted[0].1["items"][0]["amount"]["value"], "12.50");

    let (status, _) = body(create_payout(State(state.clone()), origin(), Json(payout("sb-buyer@personal.example.com"))).await).await;
    assert_eq!(status, StatusCode::CONFLICT, "the same request ID is never sent twice");
    assert_eq!(mock.posts_to("/v1/payments/payouts").len(), 1);

    let (status, checked) = body(payout_status(State(state.clone()), Path("BATCHWS12345".to_owned())).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(checked["status"], "UNCLAIMED");
    assert_eq!(state.ledger.lock().await.data.payouts[0].status, "UNCLAIMED");
    let (status, _) = body(payout_status(State(state), Path("BATCHNOTOURS1".to_owned())).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invoices_use_the_copilot_validator_and_sync_status_from_paypal() {
    let disabled = spawn_mock("openid").await;
    let (status, listed) = body(list_invoices(State(state_for(&disabled))).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["enabled"], false);

    let mock = spawn_mock(INVOICING).await;
    let state = state_for(&mock);
    let form = |recipient: &str| {
        parse::<InvoiceForm>(json!({
            "recipient": recipient,
            "items": [{ "name": "Consulting", "quantity": "3", "unitPrice": "60", "hours": true }, { "name": "Setup", "quantity": "1", "unitPrice": "25" }],
            "paymentTerm": "NET_15"
        }))
    };
    let (status, _) = body(create_invoice(State(state.clone()), origin(), Json(form("jordan@gmail.com"))).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, created) = body(create_invoice(State(state.clone()), origin(), Json(form("Jordan@Example.com"))).await).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["invoiceId"], "INV2-WS00-0001");
    assert_eq!(created["total"], "$205.00");
    let posted = mock.posts_to("/v2/invoicing/invoices");
    assert_eq!(posted[0].1["items"][0]["unit_of_measure"], "HOURS");
    assert_eq!(posted[0].1["detail"]["payment_term"]["term_type"], "NET_15");

    // A draft cannot be cancelled here; it must be sent first.
    let (status, _) = body(invoice_action(State(state.clone()), origin(), Path(("INV2-WS00-0001".to_owned(), "cancel".to_owned()))).await).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // PayPal reports it paid: the record follows and the audit trail says so.
    let (_, listed) = body(list_invoices(State(state.clone())).await).await;
    assert_eq!(listed["invoices"][0]["managed"], true);
    assert_eq!(listed["invoices"][0]["canSend"], false);
    assert_eq!(listed["invoices"][0]["payUrl"], "https://www.sandbox.paypal.com/invoice/p/#INV2WS000001");
    {
        let ledger = state.ledger.lock().await;
        assert_eq!(ledger.data.invoices[0].status, "PAID");
        assert_eq!(ledger.data.events.last().unwrap().title, "Invoice paid");
    }

    // Cancel works on an open invoice and accepts PayPal's empty 204 reply.
    state.ledger.lock().await.data.invoices[0].status = "sent".to_owned();
    let (status, _) = body(invoice_action(State(state.clone()), origin(), Path(("INV2-WS00-0001".to_owned(), "cancel".to_owned()))).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mock.posts_to("/v2/invoicing/invoices/INV2-WS00-0001/cancel").len(), 1);
    assert_eq!(state.ledger.lock().await.data.invoices[0].status, "cancelled");
    let (status, _) = body(invoice_action(State(state.clone()), origin(), Path(("INV2-WS00-0001".to_owned(), "delete".to_owned()))).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn profile_and_range_inputs_are_validated() {
    let mock = spawn_mock("openid").await;
    let state = state_for(&mock);
    let (status, _) = body(
        update_profile(
            State(state.clone()),
            origin(),
            Json(parse(json!({ "displayName": "", "businessName": "Shop" }))),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(serde_json::from_value::<ProfileUpdate>(json!({ "displayName": "A", "businessName": "B", "admin": true })).is_err());
    let (status, _) = body(dashboard(State(state.clone()), Query(RangeQuery { days: Some(365) })).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, figures) = body(dashboard(State(state), Query(RangeQuery { days: Some(7) })).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(figures["days"], 7);
}
