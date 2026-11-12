//! Workspace pages backed by PayPal Sandbox: dashboard, activity, wallet, profile, Send &
//! request (payment requests and Payouts), invoices, and refunds.
//!
//! Every figure comes from the persistent record of what this app did with PayPal (see
//! `ledger`), or from a live PayPal call. Nothing here is sample data.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AppState, OrderState,
    copilot::{
        self, ActionKind, Capabilities, clean_text, decimal, now_seconds, optional_text, usd,
        utc_timestamp,
    },
    create_invoice_draft, create_paypal_checkout_order, ledger, paypal_api_request,
    paypal_api_request_with_id, paypal_token, same_origin, send_sandbox_invoice,
    valid_approval_url, valid_invoice_id, valid_test_recipient,
};

const DAY: i64 = 86_400;
const MAX_CENTS: i64 = 999_999;
const ENABLE_HINT: &str = "In the PayPal Developer Dashboard, open Apps & Credentials, select the Sandbox app, tick the feature under Features, save, and try again in a few minutes.";

/// Payout statuses that mean the money did not (or will not) leave the account.
const PAYOUT_FAILED: &[&str] = &[
    "DENIED", "CANCELED", "FAILED", "RETURNED", "REFUNDED", "REVERSED", "BLOCKED",
];

fn reply(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    reply(status, json!({ "error": message.into() }))
}

fn forbidden() -> Response {
    error(
        StatusCode::FORBIDDEN,
        "Open the app from its configured origin (APP_BASE_URL).",
    )
}

fn paypal_failure(error: crate::ApiError) -> Response {
    eprintln!("PayPal Sandbox request failed: {}", error.message);
    reply(
        StatusCode::BAD_GATEWAY,
        json!({ "error": error.message, "paypalIssue": error.issue }),
    )
}

async fn capabilities(state: &AppState) -> Result<Capabilities, crate::ApiError> {
    paypal_token(state)
        .await
        .map(|token| Capabilities::from_scope(&token.scope))
}

fn valid_request_id(value: &str) -> bool {
    value.len() == 36 && Uuid::parse_str(value).is_ok()
}

fn timestamp(seconds: i64) -> String {
    utc_timestamp(seconds)
}

fn amount_cents(text: &str) -> Option<i64> {
    copilot::hundredths_arg(&json!(text)).filter(|cents| (1..=MAX_CENTS).contains(cents))
}

// ---------------------------------------------------------------------------------------------
// Dashboard (pure, unit-tested)

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeQuery {
    days: Option<i64>,
}

#[derive(Default)]
struct Period {
    gross: i64,
    orders: i64,
    refunds: i64,
    customers: BTreeSet<String>,
    repeat: usize,
}

/// Who paid, for repeat-customer counts: the payer's email, else their name.
fn customer_key(order: &ledger::OrderRecord) -> Option<String> {
    order
        .payer_email
        .as_deref()
        .or(order.payer_name.as_deref())
        .map(str::to_ascii_lowercase)
}

fn within(at: i64, from: i64, to: i64) -> bool {
    at > from && at <= to
}

fn period(data: &ledger::Data, from: i64, to: i64) -> Period {
    let mut period = Period::default();
    let paid: Vec<&ledger::OrderRecord> = data.orders.iter().filter(|o| o.is_paid()).collect();
    for order in &paid {
        if order.captured_at.is_some_and(|at| within(at, from, to)) {
            period.gross += order.captured_cents;
            period.orders += 1;
            if let Some(customer) = customer_key(order) {
                period.customers.insert(customer);
            }
        }
    }
    for refund in data.orders.iter().flat_map(|order| &order.refunds) {
        if within(refund.at, from, to) {
            period.refunds += refund.cents;
        }
    }
    period.repeat = period
        .customers
        .iter()
        .filter(|customer| {
            paid.iter()
                .filter(|order| {
                    customer_key(order).as_deref() == Some(customer.as_str())
                        && order.captured_at.is_some_and(|at| at <= to)
                })
                .count()
                >= 2
        })
        .count();
    period
}

fn change_pct(current: i64, previous: i64) -> Value {
    if previous > 0 {
        json!(((current - previous) as f64 * 1000.0 / previous as f64).round() / 10.0)
    } else {
        Value::Null
    }
}

fn repeat_pct(period: &Period) -> Option<f64> {
    (!period.customers.is_empty())
        .then(|| (period.repeat as f64 * 1000.0 / period.customers.len() as f64).round() / 10.0)
}

fn insight(
    current: &Period,
    previous: &Period,
    any_paid: bool,
    unpaid: (usize, i64),
    days: i64,
) -> Value {
    let (title, detail, tag) = if !any_paid {
        (
            "No payments yet".to_owned(),
            "Create a payment link in Send & request, or ask Boardroom Copilot for one. Everything on this dashboard comes from real Sandbox payments.".to_owned(),
            "GET STARTED",
        )
    } else if current.orders == 0 {
        (
            format!("No payments in the last {days} days"),
            "Share a payment link or send an invoice to get paid.".to_owned(),
            "SALES",
        )
    } else if current.gross > 0 && current.refunds * 10 >= current.gross {
        (
            format!(
                "Refunds are {:.1}% of gross sales",
                current.refunds as f64 * 100.0 / current.gross as f64
            ),
            format!(
                "{} refunded against {} in sales over the last {days} days.",
                usd(current.refunds),
                usd(current.gross)
            ),
            "REFUNDS",
        )
    } else if unpaid.0 >= 2 {
        (
            format!("{} payment links are still unpaid", unpaid.0),
            format!(
                "{} is waiting on buyers. Check Activity to resend or follow up.",
                usd(unpaid.1)
            ),
            "FOLLOW UP",
        )
    } else if previous.gross > 0 && current.gross * 10 <= previous.gross * 9 {
        (
            format!(
                "Sales are down {:.0}% from the previous {days} days",
                (previous.gross - current.gross) as f64 * 100.0 / previous.gross as f64
            ),
            format!("{} now vs. {} before.", usd(current.gross), usd(previous.gross)),
            "SALES",
        )
    } else if current.customers.len() >= 3 && current.repeat * 5 < current.customers.len() {
        (
            "Most customers bought once".to_owned(),
            format!(
                "{} of {} paying customers have bought more than once.",
                current.repeat,
                current.customers.len()
            ),
            "RETENTION",
        )
    } else if previous.gross > 0 && current.gross * 10 >= previous.gross * 11 {
        (
            format!(
                "Sales are up {:.0}% from the previous {days} days",
                (current.gross - previous.gross) as f64 * 100.0 / previous.gross as f64
            ),
            format!("{} now vs. {} before.", usd(current.gross), usd(previous.gross)),
            "GROWTH",
        )
    } else {
        (
            format!(
                "{} from {} payment{}",
                usd(current.gross),
                current.orders,
                if current.orders == 1 { "" } else { "s" }
            ),
            format!("Payments captured through Boardroom in the last {days} days."),
            "OVERVIEW",
        )
    };
    json!({ "title": title, "detail": detail, "tag": tag })
}

/// Dashboard figures for the `days` ending at `now`, with the previous period for comparison.
pub(crate) fn summarize(data: &ledger::Data, now: i64, days: i64) -> Value {
    let start = now - days * DAY;
    let current = period(data, start, now);
    let previous = period(data, start - days * DAY, start);
    let any_paid = data.orders.iter().any(ledger::OrderRecord::is_paid);

    let length = days as usize;
    let mut daily_gross = vec![0i64; length];
    let mut daily_orders = vec![0i64; length];
    let mut daily_refunds = vec![0i64; length];
    let bucket = |at: i64| (((at - start - 1).max(0) / DAY) as usize).min(length - 1);
    let mut items: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for order in data.orders.iter().filter(|order| order.is_paid()) {
        if let Some(at) = order.captured_at.filter(|at| within(*at, start, now)) {
            daily_gross[bucket(at)] += order.captured_cents;
            daily_orders[bucket(at)] += 1;
            let entry = items.entry(order.description.clone()).or_default();
            entry.0 += order.captured_cents;
            entry.1 += 1;
        }
    }
    for refund in data.orders.iter().flat_map(|order| &order.refunds) {
        if within(refund.at, start, now) {
            daily_refunds[bucket(refund.at)] += refund.cents;
        }
    }
    let series: Vec<Value> = daily_gross
        .iter()
        .enumerate()
        .map(|(index, cents)| {
            let day_end = (start + (index as i64 + 1) * DAY).min(now);
            json!({ "date": &timestamp(day_end)[..10], "cents": cents })
        })
        .collect();
    // Twelve equal slices for the small trend bars on each metric card.
    let buckets: Vec<Value> = (0..12)
        .map(|slice| {
            let from = slice * length / 12;
            let to = ((slice + 1) * length / 12).max(from + 1).min(length);
            let sum = |values: &[i64]| values[from..to].iter().sum::<i64>();
            json!({ "gross": sum(&daily_gross), "orders": sum(&daily_orders), "refunds": sum(&daily_refunds) })
        })
        .collect();
    let mut top: Vec<(String, (i64, i64))> = items.into_iter().collect();
    top.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    let top_items: Vec<Value> = top
        .into_iter()
        .take(3)
        .map(|(name, (cents, orders))| json!({ "name": name, "cents": cents, "text": usd(cents), "orders": orders }))
        .collect();
    let unpaid: Vec<&ledger::OrderRecord> = data
        .orders
        .iter()
        .filter(|order| order.state == "created" && order.source != "card")
        .filter(|order| within(order.created_at, start, now))
        .collect();
    let unpaid_cents: i64 = unpaid.iter().map(|order| order.amount_cents).sum();
    let average = |period: &Period| (period.orders > 0).then(|| period.gross / period.orders);
    let net = current.gross - current.refunds;

    json!({
        "days": days,
        "source": "Payments captured through Boardroom on PayPal Sandbox",
        "generatedAt": timestamp(now),
        "hasData": any_paid,
        "gross": { "cents": current.gross, "text": usd(current.gross), "changePct": change_pct(current.gross, previous.gross) },
        "orders": { "count": current.orders, "previous": previous.orders },
        "averageOrder": {
            "cents": average(&current),
            "text": average(&current).map(usd),
            "changePct": match (average(&current), average(&previous)) {
                (Some(now), Some(before)) => change_pct(now, before),
                _ => Value::Null,
            }
        },
        "repeatCustomers": {
            "pct": repeat_pct(&current),
            "customers": current.customers.len(),
            "repeat": current.repeat,
            "changePts": match (repeat_pct(&current), repeat_pct(&previous)) {
                (Some(now), Some(before)) => json!(((now - before) * 10.0).round() / 10.0),
                _ => Value::Null,
            }
        },
        "refunds": {
            "cents": current.refunds,
            "text": usd(current.refunds),
            "pctOfGross": if current.gross > 0 {
                json!((current.refunds as f64 * 1000.0 / current.gross as f64).round() / 10.0)
            } else {
                Value::Null
            }
        },
        "net": { "cents": net, "text": usd(net) },
        "unpaid": { "count": unpaid.len(), "cents": unpaid_cents, "text": usd(unpaid_cents) },
        "series": series,
        "buckets": buckets,
        "topItems": top_items,
        "insight": insight(&current, &previous, any_paid, (unpaid.len(), unpaid_cents), days)
    })
}

pub async fn dashboard(State(state): State<AppState>, Query(query): Query<RangeQuery>) -> Response {
    let days = match query.days.unwrap_or(30) {
        days @ (7 | 30 | 90) => days,
        _ => return error(StatusCode::BAD_REQUEST, "days must be 7, 30, or 90."),
    };
    let ledger = state.ledger.lock().await;
    reply(StatusCode::OK, summarize(&ledger.data, now_seconds(), days))
}

// ---------------------------------------------------------------------------------------------
// Activity and audit trail

fn order_status(order: &ledger::OrderRecord) -> &'static str {
    match order.state.as_str() {
        "created" => "Awaiting payment",
        "capture_pending" => "Capturing…",
        "capture_unknown" => "Check Sandbox",
        "captured" if order.refunded_cents() >= order.captured_cents => "Refunded",
        "captured" if order.refunded_cents() > 0 => "Partially refunded",
        "captured" => "Paid",
        _ => "Unknown",
    }
}

fn order_json(order: &ledger::OrderRecord) -> Value {
    let refundable = if order.is_paid() {
        (order.captured_cents - order.refunded_cents()).max(0)
    } else {
        0
    };
    let approval_url = order
        .approval_url
        .as_deref()
        .filter(|url| order.state == "created" && valid_approval_url(url));
    json!({
        "id": order.id,
        "createdAt": timestamp(order.created_at),
        "capturedAt": order.captured_at.map(timestamp),
        "source": order.source,
        "description": order.description,
        "customer": order.customer,
        "payer": order.payer_name.as_deref().or(order.payer_email.as_deref()),
        "payerEmail": order.payer_email,
        "amount": usd(order.amount_cents),
        "captured": usd(order.captured_cents),
        "refunded": usd(order.refunded_cents()),
        "refundableCents": refundable,
        "refundable": decimal(refundable),
        "status": order_status(order),
        "state": order.state,
        "approvalUrl": approval_url
    })
}

pub async fn activity(State(state): State<AppState>) -> Response {
    let ledger = state.ledger.lock().await;
    let data = &ledger.data;
    let orders: Vec<Value> = data.orders.iter().rev().take(50).map(order_json).collect();
    let payouts: Vec<Value> = data
        .payouts
        .iter()
        .rev()
        .take(50)
        .map(|payout| {
            json!({
                "batchId": payout.batch_id,
                "createdAt": timestamp(payout.created_at),
                "recipient": payout.recipient,
                "amount": usd(payout.amount_cents),
                "note": payout.note,
                "status": payout.status
            })
        })
        .collect();
    let invoices: Vec<Value> = data
        .invoices
        .iter()
        .rev()
        .take(50)
        .map(|invoice| {
            json!({
                "id": invoice.id,
                "createdAt": timestamp(invoice.created_at),
                "recipient": invoice.recipient,
                "amount": usd(invoice.amount_cents),
                "status": invoice.status
            })
        })
        .collect();
    let events: Vec<Value> = data
        .events
        .iter()
        .rev()
        .take(100)
        .map(|event| json!({ "at": timestamp(event.at), "kind": event.kind, "title": event.title, "detail": event.detail }))
        .collect();
    reply(
        StatusCode::OK,
        json!({ "orders": orders, "payouts": payouts, "invoices": invoices, "events": events }),
    )
}

// ---------------------------------------------------------------------------------------------
// Wallet

/// Money in and out through this app, all time: captured − refunded − paid out.
pub(crate) fn boardroom_balance(data: &ledger::Data) -> Value {
    let captured: i64 = data
        .orders
        .iter()
        .filter(|order| order.is_paid())
        .map(|order| order.captured_cents)
        .sum();
    let refunded: i64 = data.orders.iter().map(ledger::OrderRecord::refunded_cents).sum();
    let paid_out: i64 = data
        .payouts
        .iter()
        .filter(|payout| !PAYOUT_FAILED.contains(&payout.status.as_str()))
        .map(|payout| payout.amount_cents)
        .sum();
    let net = captured - refunded - paid_out;
    json!({
        "captured": usd(captured),
        "refunded": usd(refunded),
        "paidOut": usd(paid_out),
        "net": usd(net),
        "netCents": net,
        "payments": data.orders.iter().filter(|order| order.is_paid()).count()
    })
}

pub async fn wallet(State(state): State<AppState>) -> Response {
    let boardroom = boardroom_balance(&state.ledger.lock().await.data);
    let paypal = match capabilities(&state).await {
        Err(error) => json!({ "available": false, "error": error.message }),
        Ok(capabilities) if !capabilities.transaction_search => json!({
            "available": false,
            "error": format!("Showing the PayPal account balance needs the Transaction search feature. {ENABLE_HINT}")
        }),
        Ok(_) => match paypal_api_request(
            &state,
            Method::GET,
            "/v1/reporting/balances?currency_code=USD",
            Value::Null,
        )
        .await
        {
            Ok(payload) => {
                let balances = payload["balances"].as_array().cloned().unwrap_or_default();
                let balance = balances
                    .iter()
                    .find(|balance| balance["currency"].as_str() == Some("USD"))
                    .or_else(|| balances.first());
                match balance {
                    Some(balance) => json!({
                        "available": true,
                        "total": copilot::money_value(&balance["total_balance"]).map(|(cents, _)| usd(cents)),
                        "availableBalance": copilot::money_value(&balance["available_balance"]).map(|(cents, _)| usd(cents)),
                        "currency": balance["currency"],
                        "asOf": payload["as_of_time"]
                    }),
                    None => json!({ "available": false, "error": "PayPal returned no balances for this Sandbox account." }),
                }
            }
            Err(error) => json!({ "available": false, "error": error.message }),
        },
    };
    reply(
        StatusCode::OK,
        json!({ "boardroom": boardroom, "paypal": paypal }),
    )
}

// ---------------------------------------------------------------------------------------------
// Profile

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProfileUpdate {
    display_name: String,
    business_name: String,
}

pub async fn profile(State(state): State<AppState>) -> Response {
    let profile = state.ledger.lock().await.data.profile.clone();
    reply(
        StatusCode::OK,
        json!({ "displayName": profile.display_name, "businessName": profile.business_name }),
    )
}

pub async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<ProfileUpdate>,
) -> Response {
    if !same_origin(&headers, &state) {
        return forbidden();
    }
    let Some(display_name) = clean_text(&json!(update.display_name), 40, false) else {
        return error(StatusCode::BAD_REQUEST, "Display name must be 1 to 40 characters.");
    };
    let Some(business_name) = clean_text(&json!(update.business_name), 60, false) else {
        return error(StatusCode::BAD_REQUEST, "Business name must be 1 to 60 characters.");
    };
    let mut ledger = state.ledger.lock().await;
    ledger.data.profile = ledger::Profile {
        display_name: display_name.clone(),
        business_name: business_name.clone(),
    };
    ledger.event(
        "settings",
        "Profile updated",
        format!("{display_name} · checkout brand “{business_name}”"),
    );
    ledger.save();
    reply(
        StatusCode::OK,
        json!({ "displayName": display_name, "businessName": business_name }),
    )
}

// ---------------------------------------------------------------------------------------------
// Request money (Orders v2 checkout link)

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PaymentRequest {
    amount: String,
    description: String,
    #[serde(default)]
    customer: Option<String>,
}

pub async fn request_payment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PaymentRequest>,
) -> Response {
    if !same_origin(&headers, &state) {
        return forbidden();
    }
    let (amount_cents, description) = match copilot::validate_payment_link(
        &json!({ "amount": request.amount, "description": request.description }),
    ) {
        Ok(ActionKind::PaymentLink {
            amount_cents,
            description,
        }) => (amount_cents, description),
        Ok(_) => unreachable!("validate_payment_link only builds payment links"),
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    let customer = match optional_text(&json!(request.customer), 80, false) {
        Ok(customer) => customer,
        Err(()) => return error(StatusCode::BAD_REQUEST, "Customer must be at most 80 characters."),
    };
    let request_id = Uuid::new_v4().to_string();
    match create_paypal_checkout_order(&state, amount_cents, &description, &request_id, "request")
        .await
    {
        Ok((order_id, approval_url)) => {
            if customer.is_some() {
                let mut ledger = state.ledger.lock().await;
                if let Some(order) = ledger.order_mut(&order_id) {
                    order.customer = customer;
                }
                ledger.save();
            }
            reply(
                StatusCode::CREATED,
                json!({ "orderId": order_id, "approvalUrl": approval_url, "amount": usd(amount_cents) }),
            )
        }
        Err(error) => paypal_failure(error),
    }
}

// ---------------------------------------------------------------------------------------------
// Refund a captured order

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RefundRequest {
    amount: String,
    #[serde(default)]
    note: Option<String>,
    request_id: String,
}

pub async fn refund_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
    Json(request): Json<RefundRequest>,
) -> Response {
    if !same_origin(&headers, &state) || !crate::valid_order_id(&order_id) {
        return forbidden();
    }
    if !valid_request_id(&request.request_id) {
        return error(StatusCode::BAD_REQUEST, "requestId must be a UUID.");
    }
    if state.orders.lock().await.get(&order_id) != Some(&OrderState::Captured) {
        return error(
            StatusCode::CONFLICT,
            "Only orders this app created and captured can be refunded here.",
        );
    }
    let Some(cents) = amount_cents(&request.amount) else {
        return error(
            StatusCode::BAD_REQUEST,
            "Enter a refund between $0.01 and the refundable balance.",
        );
    };
    let note = match optional_text(&json!(request.note), 255, false) {
        Ok(note) => note,
        Err(()) => return error(StatusCode::BAD_REQUEST, "Note must be at most 255 characters."),
    };
    match copilot::execute_refund(&state, &request.request_id, &order_id, cents, note.as_deref())
        .await
    {
        Ok(result) => reply(StatusCode::OK, result),
        Err(message) => error(StatusCode::BAD_GATEWAY, message),
    }
}

// ---------------------------------------------------------------------------------------------
// Send money (Payouts API)

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PayoutRequest {
    recipient: String,
    amount: String,
    #[serde(default)]
    note: Option<String>,
    request_id: String,
}

pub(crate) fn payout_body(
    request_id: &str,
    recipient: &str,
    cents: i64,
    note: Option<&str>,
    business: &str,
) -> Value {
    let message = note.map_or_else(|| format!("Payment from {business}"), str::to_owned);
    json!({
        "sender_batch_header": {
            "sender_batch_id": request_id,
            "email_subject": format!("You have a payment from {business}"),
            "email_message": message
        },
        "items": [{
            "recipient_type": "EMAIL",
            "amount": { "value": decimal(cents), "currency": "USD" },
            "receiver": recipient,
            "note": message,
            "sender_item_id": format!("item-{}", &request_id[..8])
        }]
    })
}

fn valid_batch_id(value: &str) -> bool {
    (6..=64).contains(&value.len()) && value.bytes().all(|ch| ch.is_ascii_alphanumeric())
}

pub async fn create_payout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PayoutRequest>,
) -> Response {
    if !same_origin(&headers, &state) {
        return forbidden();
    }
    match capabilities(&state).await {
        Ok(capabilities) if capabilities.payouts => {}
        Ok(_) => {
            return error(
                StatusCode::FORBIDDEN,
                format!("This Sandbox app is not permitted to send Payouts. {ENABLE_HINT}"),
            );
        }
        Err(error) => return paypal_failure(error),
    }
    let recipient = request.recipient.trim().to_ascii_lowercase();
    if !valid_test_recipient(&recipient) {
        return error(
            StatusCode::BAD_REQUEST,
            "Send only to a test address at example.com, such as a Sandbox account (sb-…@personal.example.com).",
        );
    }
    let Some(cents) = amount_cents(&request.amount) else {
        return error(
            StatusCode::BAD_REQUEST,
            "Enter an amount between $0.01 and $9,999.99.",
        );
    };
    let note = match optional_text(&json!(request.note), 255, false) {
        Ok(note) => note,
        Err(()) => return error(StatusCode::BAD_REQUEST, "Note must be at most 255 characters."),
    };
    if !valid_request_id(&request.request_id) {
        return error(StatusCode::BAD_REQUEST, "requestId must be a UUID.");
    }
    let business = {
        let ledger = state.ledger.lock().await;
        if ledger
            .data
            .payouts
            .iter()
            .any(|payout| payout.request_id == request.request_id)
        {
            return error(
                StatusCode::CONFLICT,
                "This payout was already submitted. Check Activity for its status.",
            );
        }
        ledger.data.profile.business_name.clone()
    };
    let body = payout_body(&request.request_id, &recipient, cents, note.as_deref(), &business);
    let result = match paypal_api_request_with_id(
        &state,
        Method::POST,
        "/v1/payments/payouts",
        body,
        &request.request_id,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => return paypal_failure(error),
    };
    let Some(batch_id) = result["batch_header"]["payout_batch_id"]
        .as_str()
        .filter(|id| valid_batch_id(id))
        .map(str::to_owned)
    else {
        return error(
            StatusCode::BAD_GATEWAY,
            "PayPal did not return a usable payout batch ID. Check Sandbox activity before retrying.",
        );
    };
    let status = result["batch_header"]["batch_status"]
        .as_str()
        .unwrap_or("PENDING")
        .to_owned();
    let mut ledger = state.ledger.lock().await;
    ledger.data.payouts.push(ledger::PayoutRecord {
        batch_id: batch_id.clone(),
        request_id: request.request_id.clone(),
        created_at: now_seconds(),
        recipient: recipient.clone(),
        amount_cents: cents,
        note,
        status: status.clone(),
    });
    ledger.event(
        "payout",
        "Payout sent",
        format!("{} to {recipient} · batch {batch_id}", usd(cents)),
    );
    ledger.save();
    reply(
        StatusCode::CREATED,
        json!({ "batchId": batch_id, "status": status, "amount": usd(cents), "recipient": recipient }),
    )
}

pub async fn payout_status(State(state): State<AppState>, Path(batch_id): Path<String>) -> Response {
    if !valid_batch_id(&batch_id) {
        return error(StatusCode::BAD_REQUEST, "Invalid payout batch ID.");
    }
    if state.ledger.lock().await.payout_mut(&batch_id).is_none() {
        return error(StatusCode::NOT_FOUND, "This app did not send that payout.");
    }
    let result = match paypal_api_request(
        &state,
        Method::GET,
        &format!("/v1/payments/payouts/{batch_id}"),
        Value::Null,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => return paypal_failure(error),
    };
    let batch_status = result["batch_header"]["batch_status"].as_str();
    let item_status = result["items"][0]["transaction_status"].as_str();
    let status = item_status.or(batch_status).unwrap_or("UNKNOWN").to_owned();
    let mut ledger = state.ledger.lock().await;
    if let Some(payout) = ledger.payout_mut(&batch_id)
        && payout.status != status
    {
        payout.status = status.clone();
        let detail = format!("{} to {} · {status}", usd(payout.amount_cents), payout.recipient);
        ledger.event("payout", "Payout status updated", detail);
        ledger.save();
    }
    reply(
        StatusCode::OK,
        json!({ "batchId": batch_id, "status": status, "batchStatus": batch_status, "itemStatus": item_status }),
    )
}

// ---------------------------------------------------------------------------------------------
// Invoices (Invoicing v2)

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InvoiceForm {
    recipient: String,
    items: Vec<InvoiceFormItem>,
    payment_term: String,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InvoiceFormItem {
    name: String,
    quantity: String,
    unit_price: String,
    #[serde(default)]
    hours: bool,
}

/// Converts the form to the same arguments the Copilot uses, so both share one validator.
fn invoice_args(form: &InvoiceForm) -> Value {
    json!({
        "recipient_email": form.recipient.trim().to_ascii_lowercase(),
        "items": form.items.iter().map(|item| json!({
            "name": item.name,
            "quantity": item.quantity,
            "unit_price": item.unit_price,
            "unit_of_measure": if item.hours { "HOURS" } else { "QUANTITY" }
        })).collect::<Vec<_>>(),
        "payment_term": form.payment_term,
        "note": form.note
    })
}

async fn require_invoicing(state: &AppState) -> Result<(), Response> {
    match capabilities(state).await {
        Ok(capabilities) if capabilities.invoicing => Ok(()),
        Ok(_) => Err(error(
            StatusCode::FORBIDDEN,
            format!("This Sandbox app is not permitted to use Invoicing. {ENABLE_HINT}"),
        )),
        Err(error) => Err(paypal_failure(error)),
    }
}

pub async fn create_invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<InvoiceForm>,
) -> Response {
    if !same_origin(&headers, &state) {
        return forbidden();
    }
    let draft = match copilot::validate_invoice(&invoice_args(&form)) {
        Ok(draft) => draft,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    if let Err(response) = require_invoicing(&state).await {
        return response;
    }
    let request_id = Uuid::new_v4().to_string();
    match create_invoice_draft(
        &state,
        copilot::invoice_body(&draft),
        &request_id,
        &draft.recipient,
        draft.total_cents(),
    )
    .await
    {
        Ok(invoice_id) => reply(
            StatusCode::CREATED,
            json!({ "invoiceId": invoice_id, "total": usd(draft.total_cents()), "recipient": draft.recipient }),
        ),
        Err(error) => paypal_failure(error),
    }
}

/// Our ledger status for an invoice after PayPal reports `paypal_status`.
fn synced_status(local: &str, paypal_status: &str) -> Option<String> {
    let updated = match paypal_status {
        "DRAFT" => return None,
        "SENT" | "UNPAID" if matches!(local, "sent" | "send_unknown" | "send_pending") => {
            return None;
        }
        "CANCELLED" => "cancelled".to_owned(),
        other => other.to_owned(),
    };
    (updated != local).then_some(updated)
}

pub async fn list_invoices(State(state): State<AppState>) -> Response {
    match capabilities(&state).await {
        Ok(capabilities) if capabilities.invoicing => {}
        Ok(_) => {
            return reply(
                StatusCode::OK,
                json!({ "enabled": false, "message": format!("Invoicing is not enabled for this Sandbox app. {ENABLE_HINT}"), "invoices": [] }),
            );
        }
        Err(error) => return paypal_failure(error),
    }
    let payload = match paypal_api_request(
        &state,
        Method::GET,
        "/v2/invoicing/invoices?page=1&page_size=50&total_required=true",
        Value::Null,
    )
    .await
    {
        Ok(payload) => payload,
        Err(error) => return paypal_failure(error),
    };
    let items = payload["items"].as_array().cloned().unwrap_or_default();
    let drafts = state.invoices.lock().await.clone();
    let mut ledger = state.ledger.lock().await;
    let mut changed = false;
    let rows: Vec<Value> = items
        .iter()
        .filter_map(|invoice| {
            let id = invoice["id"].as_str().filter(|id| valid_invoice_id(id))?;
            let status = invoice["status"].as_str().unwrap_or("UNKNOWN");
            let mut paid_detail = None;
            let managed = match ledger.invoice_mut(id) {
                Some(record) => {
                    if let Some(updated) = synced_status(&record.status, status) {
                        if matches!(updated.as_str(), "PAID" | "MARKED_AS_PAID") {
                            paid_detail = Some(format!(
                                "{id} · {} · {}",
                                usd(record.amount_cents),
                                record.recipient
                            ));
                        }
                        record.status = updated;
                        changed = true;
                    }
                    true
                }
                None => false,
            };
            if let Some(detail) = paid_detail {
                ledger.event("invoice", "Invoice paid", detail);
            }
            let open = matches!(status, "SENT" | "UNPAID" | "PARTIALLY_PAID" | "PAYMENT_PENDING");
            let pay_url = invoice["detail"]["metadata"]["recipient_view_url"]
                .as_str()
                .filter(|url| valid_approval_url(url));
            Some(json!({
                "id": id,
                "number": invoice["detail"]["invoice_number"],
                "status": status,
                "recipient": invoice["primary_recipients"][0]["billing_info"]["email_address"],
                "invoiceDate": invoice["detail"]["invoice_date"],
                "dueDate": invoice["detail"]["payment_term"]["due_date"],
                "total": copilot::money_value(&invoice["amount"]).map(|(cents, _)| usd(cents)),
                "due": copilot::money_value(&invoice["due_amount"]).map(|(cents, _)| usd(cents)),
                "managed": managed,
                "canSend": managed && status == "DRAFT" && drafts.get(id) == Some(&crate::InvoiceState::Draft),
                "canRemind": managed && open,
                "canCancel": managed && open,
                "payUrl": pay_url
            }))
        })
        .collect();
    if changed {
        ledger.save();
    }
    reply(
        StatusCode::OK,
        json!({ "enabled": true, "invoices": rows, "total": payload["total_items"] }),
    )
}

pub async fn invoice_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((invoice_id, action)): Path<(String, String)>,
) -> Response {
    if action == "send" {
        return send_sandbox_invoice(State(state), headers, Path(invoice_id)).await;
    }
    if !same_origin(&headers, &state) || !valid_invoice_id(&invoice_id) {
        return forbidden();
    }
    let (path, body, title) = match action.as_str() {
        "remind" => (
            "remind",
            json!({ "note": "A friendly reminder that this invoice is open.", "send_to_invoicer": false, "send_to_recipient": true }),
            "Invoice reminder sent",
        ),
        "cancel" => (
            "cancel",
            json!({ "note": "This invoice was cancelled.", "send_to_invoicer": false, "send_to_recipient": true }),
            "Invoice cancelled",
        ),
        _ => return error(StatusCode::NOT_FOUND, "Unknown invoice action."),
    };
    {
        let mut ledger = state.ledger.lock().await;
        match ledger.invoice_mut(&invoice_id) {
            Some(record) if !matches!(record.status.as_str(), "draft" | "cancelled") => {}
            Some(_) => {
                return error(
                    StatusCode::CONFLICT,
                    "Only sent, open invoices can be reminded or cancelled.",
                );
            }
            None => {
                return error(
                    StatusCode::NOT_FOUND,
                    "Only invoices created by this app can be changed here.",
                );
            }
        }
    }
    if let Err(response) = require_invoicing(&state).await {
        return response;
    }
    if let Err(error) = paypal_api_request(
        &state,
        Method::POST,
        &format!("/v2/invoicing/invoices/{invoice_id}/{path}"),
        body,
    )
    .await
    {
        return paypal_failure(error);
    }
    let mut ledger = state.ledger.lock().await;
    if action == "cancel"
        && let Some(record) = ledger.invoice_mut(&invoice_id)
    {
        record.status = "cancelled".to_owned();
    }
    ledger.event("invoice", title, invoice_id.clone());
    ledger.save();
    reply(
        StatusCode::OK,
        json!({ "invoiceId": invoice_id, "action": action }),
    )
}

#[cfg(test)]
mod tests;
