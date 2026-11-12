mod boardroom;
mod copilot;
mod ledger;
mod llm;
mod workspace;

use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex};
use tower_http::services::ServeDir;
use uuid::Uuid;

const PAYPAL_API: &str = "https://api-m.sandbox.paypal.com";
const DEFAULT_NEGATIVE_TEST_TYPE: &str = "INSUFFICIENT_FUNDS";
const ALLOWED_NEGATIVE_TEST_TYPES: &[&str] = &[
    "INSUFFICIENT_FUNDS",
    "INSTRUMENT_DECLINED",
    "TRANSACTION_REFUSED",
    "INTERNAL_SERVER_ERROR",
    "DUPLICATE_INVOICE_ID",
];

#[derive(Clone)]
struct AppState {
    client: Client,
    port: u16,
    /// PayPal REST base URL. Always the Sandbox in the running app; tests point it at a mock.
    paypal_api: String,
    client_id: String,
    client_secret: String,
    app_base_url: String,
    public_directory: PathBuf,
    negative_testing_enabled: bool,
    negative_test_type: String,
    orders: Arc<Mutex<HashMap<String, OrderState>>>,
    /// Order IDs created by this process, oldest first, so the copilot can report recent ones.
    order_log: Arc<Mutex<Vec<String>>>,
    invoices: Arc<Mutex<HashMap<String, InvoiceState>>>,
    token_cache: Arc<Mutex<Option<CachedToken>>>,
    ai: Option<Arc<llm::AiConfig>>,
    copilot: Arc<Mutex<copilot::ActionBook>>,
    /// Persistent record of orders, captures, refunds, invoices, payouts, and audit events.
    ledger: Arc<Mutex<ledger::Store>>,
    /// Held while the AI writes a Boardroom brief, so only one runs at a time.
    brief_busy: Arc<Mutex<()>>,
}

#[derive(Clone)]
struct CachedToken {
    access_token: String,
    scope: String,
    expires_at: Instant,
}

/// Longest time a Sandbox token is reused. Short enough that permissions newly enabled in the
/// Developer Dashboard (which change the token's scopes) are picked up without a restart.
const TOKEN_REUSE_LIMIT: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OrderState {
    Created { verification: VerificationMode },
    CapturePending,
    Captured,
    CaptureUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InvoiceState {
    Draft,
    SendPending,
    Sent,
    SendUnknown,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionQuery {
    start: String,
    end: String,
}

fn valid_timestamp(value: &str) -> bool {
    value.len() == 20
        && value.bytes().enumerate().all(|(index, ch)| match index {
            4 | 7 => ch == b'-',
            10 => ch == b'T',
            13 | 16 => ch == b':',
            19 => ch == b'Z',
            _ => ch.is_ascii_digit(),
        })
}

/// Test recipients only: example.com (reserved for documentation by RFC 2606) or one of its
/// subdomains, which is where PayPal creates Sandbox accounts (sb-abc123@personal.example.com).
fn valid_test_recipient(value: &str) -> bool {
    let Some((local, domain)) = value.rsplit_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'-'))
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..");
    let label_ok = |label: &str| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    let domain_ok = domain == "example.com"
        || domain
            .strip_suffix(".example.com")
            .is_some_and(|subdomain| subdomain.len() <= 100 && subdomain.split('.').all(label_ok));
    local_ok && domain_ok
}

fn valid_invoice_id(value: &str) -> bool {
    value.starts_with("INV2-")
        && value.len() <= 64
        && value
            .bytes()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == b'-')
}

fn same_origin(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        == Some(state.app_base_url.as_str())
}

async fn sandbox_transactions(
    State(state): State<AppState>,
    Query(query): Query<TransactionQuery>,
) -> Response {
    if !valid_timestamp(&query.start) || !valid_timestamp(&query.end) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Use UTC timestamps in YYYY-MM-DDTHH:MM:SSZ format." })),
        )
            .into_response();
    }
    let mut url = match Url::parse(&format!("{PAYPAL_API}/v1/reporting/transactions")) {
        Ok(url) => url,
        Err(_) => {
            return api_error(ApiError::new(
                "Could not build the Sandbox transaction request.",
            ));
        }
    };
    url.query_pairs_mut()
        .append_pair("start_date", &query.start)
        .append_pair("end_date", &query.end)
        .append_pair("page_size", "20");
    let path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
    match paypal_api_request(&state, Method::GET, &path, json!(null)).await {
        Ok(result) => {
            let Some(details) = result["transaction_details"].as_array() else {
                return api_error(ApiError::new(
                    "PayPal returned an invalid transaction list.",
                ));
            };
            let transactions: Vec<Value> = details
                .iter()
                .map(|item| {
                    let info = &item["transaction_info"];
                    json!({
                        "id": info["transaction_id"],
                        "date": info["transaction_initiation_date"],
                        "status": info["transaction_status"],
                        "amount": info["transaction_amount"]
                    })
                })
                .collect();
            (
                [(header::CACHE_CONTROL, "no-store")],
                Json(json!({ "transactions": transactions, "totalPages": result["total_pages"] })),
            )
                .into_response()
        }
        Err(error) => api_error(error),
    }
}

/// Creates a draft invoice and remembers it so the send endpoint will accept it once.
async fn create_invoice_draft(
    state: &AppState,
    body: Value,
    request_id: &str,
    recipient: &str,
    amount_cents: i64,
) -> Result<String, ApiError> {
    let result = paypal_api_request_with_id(
        state,
        Method::POST,
        "/v2/invoicing/invoices",
        body,
        request_id,
    )
    .await?;
    let invoice_id =
        invoice_id_from_create_response(&result, &state.paypal_api).ok_or_else(|| {
            ApiError::new(
                "PayPal did not return a usable invoice ID. Check Sandbox before retrying.",
            )
        })?;
    state
        .invoices
        .lock()
        .await
        .insert(invoice_id.clone(), InvoiceState::Draft);
    let mut ledger = state.ledger.lock().await;
    ledger.data.invoices.push(ledger::InvoiceRecord {
        id: invoice_id.clone(),
        created_at: copilot::now_seconds(),
        recipient: recipient.to_owned(),
        amount_cents,
        status: "draft".to_owned(),
    });
    ledger.event(
        "invoice",
        "Invoice draft created",
        format!("{invoice_id} · {} · {recipient}", copilot::usd(amount_cents)),
    );
    ledger.save();
    Ok(invoice_id)
}

/// Updates an invoice's send state in memory and in the ledger.
async fn mark_invoice(state: &AppState, invoice_id: &str, invoice_state: InvoiceState) {
    state
        .invoices
        .lock()
        .await
        .insert(invoice_id.to_owned(), invoice_state);
    let mut ledger = state.ledger.lock().await;
    if let Some(invoice) = ledger.invoice_mut(invoice_id) {
        invoice.status = match invoice_state {
            InvoiceState::Draft => "draft",
            InvoiceState::SendPending => "send_pending",
            InvoiceState::Sent => "sent",
            InvoiceState::SendUnknown => "send_unknown",
        }
        .to_owned();
    }
    if invoice_state == InvoiceState::Sent {
        ledger.event("invoice", "Invoice sent", invoice_id.to_owned());
    }
    ledger.save();
}

/// The customer-facing PayPal page for an invoice, when PayPal reports one on its own domain.
async fn invoice_pay_url(state: &AppState, invoice_id: &str) -> Option<String> {
    let invoice = paypal_api_request(
        state,
        Method::GET,
        &format!("/v2/invoicing/invoices/{invoice_id}"),
        Value::Null,
    )
    .await
    .ok()?;
    invoice["detail"]["metadata"]["recipient_view_url"]
        .as_str()
        .filter(|url| valid_approval_url(url))
        .map(str::to_owned)
}

/// PayPal answers a draft creation with a link to the new invoice; accept only Sandbox links.
fn invoice_id_from_create_response(result: &Value, paypal_api: &str) -> Option<String> {
    if let Some(id) = result["id"].as_str().filter(|id| valid_invoice_id(id)) {
        return Some(id.to_owned());
    }
    let url = Url::parse(result["href"].as_str()?).ok()?;
    let configured = Url::parse(paypal_api).ok()?;
    let trusted_host = matches!(
        url.host_str(),
        Some("api-m.sandbox.paypal.com" | "api.sandbox.paypal.com")
    ) && url.scheme() == "https";
    let configured_host = url.origin() == configured.origin();
    if !trusted_host && !configured_host {
        return None;
    }
    url.path_segments()?
        .next_back()
        .filter(|id| valid_invoice_id(id))
        .map(str::to_owned)
}

async fn send_sandbox_invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(invoice_id): Path<String>,
) -> Response {
    if !same_origin(&headers, &state) || !valid_invoice_id(&invoice_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Invalid origin or invoice ID." })),
        )
            .into_response();
    }
    {
        let mut invoices = state.invoices.lock().await;
        if invoices.get(&invoice_id) != Some(&InvoiceState::Draft) {
            return (StatusCode::CONFLICT, Json(json!({ "error": "Only a draft created by this app can be sent, and only once." }))).into_response();
        }
        invoices.insert(invoice_id.clone(), InvoiceState::SendPending);
    }
    let token = match paypal_access_token(&state).await {
        Ok(token) => token,
        Err(error) => {
            mark_invoice(&state, &invoice_id, InvoiceState::Draft).await;
            return api_error(error);
        }
    };
    let response = state
        .client
        .post(format!(
            "{}/v2/invoicing/invoices/{invoice_id}/send",
            state.paypal_api
        ))
        .bearer_auth(token)
        .header("PayPal-Request-Id", Uuid::new_v4().to_string())
        .json(&json!({ "send_to_recipient": true, "send_to_invoicer": false }))
        .send()
        .await;
    let success = match response {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            eprintln!(
                "PayPal Sandbox invoice send returned HTTP {}.",
                response.status()
            );
            false
        }
        Err(error) => {
            eprintln!("PayPal Sandbox invoice send failed: {error}");
            false
        }
    };
    mark_invoice(
        &state,
        &invoice_id,
        if success {
            InvoiceState::Sent
        } else {
            InvoiceState::SendUnknown
        },
    )
    .await;
    if success {
        let pay_url = invoice_pay_url(&state, &invoice_id).await;
        Json(json!({ "status": "sent", "payUrl": pay_url })).into_response()
    } else {
        api_error(ApiError::new(
            "Invoice send could not be confirmed. Check Sandbox before retrying.",
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerificationMode {
    PayPalApproval,
    CardWhenRequired,
    CardAlways,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CardScenario {
    Success,
    Decline,
    ThreeDs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CardOrderRequest {
    scenario: CardScenario,
}

struct ApiError {
    message: &'static str,
    /// PayPal's error `name` (for example NOT_AUTHORIZED), when PayPal returned one.
    issue: Option<String>,
}

impl ApiError {
    fn new(message: &'static str) -> Self {
        Self {
            message,
            issue: None,
        }
    }
}

fn api_error(error: ApiError) -> Response {
    eprintln!("PayPal Sandbox request failed: {}", error.message);
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "error": error.message })),
    )
        .into_response()
}

fn required_credential(name: &str) -> Result<String, String> {
    let value = env::var(name)
        .map_err(|_| format!("Set {name} in .env to credentials for a PayPal Sandbox app."))?;
    if value.trim().is_empty() || value.starts_with("your_") {
        return Err(format!(
            "Set {name} in .env to credentials for a PayPal Sandbox app."
        ));
    }
    Ok(value)
}

fn configured_base_url() -> Result<String, String> {
    let value = env::var("APP_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_owned());
    let url = Url::parse(&value).map_err(|_| "APP_BASE_URL must be an absolute URL.".to_owned())?;

    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "APP_BASE_URL must contain only an http(s) origin, such as http://localhost:3000."
                .to_owned(),
        );
    }

    Ok(value.trim_end_matches('/').to_owned())
}

fn negative_testing_config() -> Result<(bool, String), String> {
    let enabled = match env::var("ENABLE_NEGATIVE_TESTING").as_deref() {
        Ok("true") => true,
        Ok("false") | Err(_) => false,
        Ok(_) => return Err("ENABLE_NEGATIVE_TESTING must be true or false.".to_owned()),
    };
    let test_type =
        env::var("NEGATIVE_TEST_TYPE").unwrap_or_else(|_| DEFAULT_NEGATIVE_TEST_TYPE.to_owned());

    if !ALLOWED_NEGATIVE_TEST_TYPES.contains(&test_type.as_str()) {
        return Err("NEGATIVE_TEST_TYPE is not in the supported demo allowlist.".to_owned());
    }

    Ok((enabled, test_type))
}

fn valid_order_id(order_id: &str) -> bool {
    (10..=64).contains(&order_id.len())
        && order_id
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn valid_approval_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_ascii_lowercase();
            Some(url.scheme() == "https" && host.ends_with(".paypal.com"))
        })
        .unwrap_or(false)
}

/// Returns a cached Sandbox token (with its granted scopes), fetching a new one when needed.
async fn paypal_token(state: &AppState) -> Result<CachedToken, ApiError> {
    let mut cache = state.token_cache.lock().await;
    if let Some(token) = cache
        .as_ref()
        .filter(|token| token.expires_at > Instant::now())
    {
        return Ok(token.clone());
    }

    let response = state
        .client
        .post(format!("{}/v1/oauth2/token", state.paypal_api))
        .basic_auth(&state.client_id, Some(&state.client_secret))
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await
        .map_err(|_| ApiError::new("Could not connect to PayPal Sandbox."))?;

    if !response.status().is_success() {
        eprintln!("PayPal Sandbox OAuth returned HTTP {}.", response.status());
        return Err(ApiError::new(
            "PayPal Sandbox authentication failed. Check the server credentials.",
        ));
    }

    let payload: Value = response.json().await.map_err(|_| {
        ApiError::new("PayPal Sandbox returned an invalid authentication response.")
    })?;
    let access_token = payload["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError::new("PayPal Sandbox did not return an access token."))?;
    // Renew two minutes before PayPal's expiry, and never hold a token past the reuse limit.
    let lifetime = Duration::from_secs(payload["expires_in"].as_u64().unwrap_or(0))
        .saturating_sub(Duration::from_secs(120))
        .min(TOKEN_REUSE_LIMIT);
    let token = CachedToken {
        access_token: access_token.to_owned(),
        scope: payload["scope"].as_str().unwrap_or_default().to_owned(),
        expires_at: Instant::now() + lifetime,
    };
    *cache = Some(token.clone());
    Ok(token)
}

async fn paypal_access_token(state: &AppState) -> Result<String, ApiError> {
    paypal_token(state).await.map(|token| token.access_token)
}

async fn paypal_api_request(
    state: &AppState,
    method: Method,
    path: &str,
    body: Value,
) -> Result<Value, ApiError> {
    let request_id = Uuid::new_v4().to_string();
    paypal_api_request_with_id(state, method, path, body, &request_id).await
}

/// Sends a PayPal request with a caller-chosen `PayPal-Request-Id`, so a retried create or
/// refund for the same approved action is idempotent on PayPal's side.
async fn paypal_api_request_with_id(
    state: &AppState,
    method: Method,
    path: &str,
    body: Value,
    request_id: &str,
) -> Result<Value, ApiError> {
    let access_token = paypal_access_token(state).await?;
    let has_body = method != Method::GET;
    let mut request = state
        .client
        .request(method, format!("{}{path}", state.paypal_api))
        .bearer_auth(access_token)
        .header("PayPal-Request-Id", request_id);

    if has_body {
        request = request.json(&body);
    }

    if state.negative_testing_enabled && has_body {
        request = request.header(
            "PayPal-Mock-Response",
            json!({ "mock_application_codes": state.negative_test_type }).to_string(),
        );
    }

    let response = request
        .send()
        .await
        .map_err(|_| ApiError::new("Could not connect to PayPal Sandbox."))?;
    let status = response.status();
    if !status.is_success() {
        let payload = match response.json::<Value>().await {
            Ok(payload) => payload,
            Err(error) => {
                eprintln!("Could not parse PayPal Sandbox error response: {error}");
                Value::Null
            }
        };
        let issue = paypal_issue(&payload);
        eprintln!(
            "PayPal Sandbox API returned HTTP {status} for {path} (issue: {}, debug_id: {}).",
            issue.as_deref().unwrap_or("unknown"),
            payload["debug_id"].as_str().unwrap_or("none")
        );
        return Err(ApiError {
            message: paypal_error_message(&payload),
            issue,
        });
    }

    // Some PayPal actions (invoice remind and cancel) answer 204 with no body.
    let text = response
        .text()
        .await
        .map_err(|_| ApiError::new("PayPal Sandbox returned an invalid API response."))?;
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .map_err(|_| ApiError::new("PayPal Sandbox returned an invalid API response."))
}

/// The most specific issue code in a PayPal error body.
fn paypal_issue(payload: &Value) -> Option<String> {
    payload["details"][0]["issue"]
        .as_str()
        .or_else(|| payload["name"].as_str())
        .map(str::to_owned)
}

fn paypal_error_message(payload: &Value) -> &'static str {
    let declined = payload["details"].as_array().is_some_and(|details| {
        details
            .iter()
            .any(|detail| detail["issue"].as_str() == Some("INSTRUMENT_DECLINED"))
    });
    let insufficient = payload["name"].as_str() == Some("INSUFFICIENT_FUNDS")
        || payload["details"].as_array().is_some_and(|details| {
            details
                .iter()
                .any(|detail| detail["issue"].as_str() == Some("INSUFFICIENT_FUNDS"))
        });
    if declined {
        "PayPal declined the payment method. Check Sandbox activity before trying another test card."
    } else if insufficient {
        "PayPal reported insufficient funds. In Sandbox, check the account balance in the Developer Dashboard or try a smaller amount."
    } else if matches!(
        payload["name"].as_str(),
        Some("NOT_AUTHORIZED" | "PERMISSION_DENIED" | "AUTHORIZATION_ERROR")
    ) {
        "This PayPal Sandbox app lacks permission for that feature. In the PayPal Developer Dashboard, open Apps & Credentials, select the Sandbox app, enable the feature (for example Invoicing or Transaction search), and try again in a few minutes."
    } else {
        "PayPal Sandbox could not complete the request. Check the server logs."
    }
}

/// Orders v2 body for a PayPal-approved checkout. Uses `payment_source.paypal.experience_context`
/// rather than the deprecated top-level `application_context`.
fn paypal_order_body(base_url: &str, value: &str, description: &str, brand: &str) -> Value {
    json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "description": description,
            "amount": { "currency_code": "USD", "value": value }
        }],
        "payment_source": {
            "paypal": {
                "experience_context": {
                    "brand_name": brand,
                    "user_action": "PAY_NOW",
                    "shipping_preference": "NO_SHIPPING",
                    "return_url": format!("{base_url}/checkout/complete"),
                    "cancel_url": format!("{base_url}/checkout/cancel")
                }
            }
        }
    })
}

/// Records an order in memory so capture (and the copilot) will recognize it.
async fn remember_order(state: &AppState, order_id: &str, order_state: OrderState) {
    state
        .orders
        .lock()
        .await
        .insert(order_id.to_owned(), order_state);
    let mut log = state.order_log.lock().await;
    log.push(order_id.to_owned());
    if log.len() > 100 {
        log.remove(0);
    }
}

fn verification_name(mode: VerificationMode) -> &'static str {
    match mode {
        VerificationMode::PayPalApproval => "paypal",
        VerificationMode::CardWhenRequired => "card_when_required",
        VerificationMode::CardAlways => "card_always",
    }
}

fn verification_from_name(name: &str) -> VerificationMode {
    match name {
        "card_when_required" => VerificationMode::CardWhenRequired,
        "card_always" => VerificationMode::CardAlways,
        _ => VerificationMode::PayPalApproval,
    }
}

fn order_state_name(order_state: OrderState) -> &'static str {
    match order_state {
        OrderState::Created { .. } => "created",
        OrderState::CapturePending => "capture_pending",
        OrderState::Captured => "captured",
        OrderState::CaptureUnknown => "capture_unknown",
    }
}

/// After a restart, an order whose capture was in flight is treated as unknown, never retried.
fn order_state_from_record(record: &ledger::OrderRecord) -> OrderState {
    match record.state.as_str() {
        "created" => OrderState::Created {
            verification: verification_from_name(&record.verification),
        },
        "captured" => OrderState::Captured,
        _ => OrderState::CaptureUnknown,
    }
}

fn invoice_state_from_record(record: &ledger::InvoiceRecord) -> InvoiceState {
    match record.status.as_str() {
        "draft" => InvoiceState::Draft,
        "send_pending" | "send_unknown" => InvoiceState::SendUnknown,
        _ => InvoiceState::Sent,
    }
}

/// Rebuilds the in-memory order and invoice state from the persistent record.
fn hydrate(
    data: &ledger::Data,
) -> (
    HashMap<String, OrderState>,
    Vec<String>,
    HashMap<String, InvoiceState>,
) {
    let orders = data
        .orders
        .iter()
        .map(|record| (record.id.clone(), order_state_from_record(record)))
        .collect();
    let skip = data.orders.len().saturating_sub(100);
    let log = data
        .orders
        .iter()
        .skip(skip)
        .map(|record| record.id.clone())
        .collect();
    let invoices = data
        .invoices
        .iter()
        .map(|record| (record.id.clone(), invoice_state_from_record(record)))
        .collect();
    (orders, log, invoices)
}

/// Records a newly created order in memory and in the persistent record.
async fn record_new_order(state: &AppState, record: ledger::OrderRecord) {
    remember_order(
        state,
        &record.id,
        OrderState::Created {
            verification: verification_from_name(&record.verification),
        },
    )
    .await;
    let title = match record.source.as_str() {
        "payment_link" => "Copilot payment link created",
        "request" => "Payment request created",
        "card" => "Card test order created",
        _ => "Checkout order created",
    };
    let mut ledger = state.ledger.lock().await;
    ledger.event(
        "order",
        title,
        format!(
            "{} · {} · order {}",
            copilot::usd(record.amount_cents),
            record.description,
            record.id
        ),
    );
    ledger.data.orders.push(record);
    ledger.save();
}

/// Updates an order's state in memory and in the persistent record.
async fn mark_order(state: &AppState, order_id: &str, order_state: OrderState) {
    state
        .orders
        .lock()
        .await
        .insert(order_id.to_owned(), order_state);
    let mut ledger = state.ledger.lock().await;
    if let Some(order) = ledger.order_mut(order_id) {
        order.state = order_state_name(order_state).to_owned();
    }
    if order_state == OrderState::CaptureUnknown {
        ledger.event(
            "payment",
            "Capture not confirmed",
            format!("Order {order_id}. Check Sandbox activity."),
        );
    }
    ledger.save();
}

/// Short, control-character-free text from a PayPal field (buyer names can be anything).
fn plain(value: Option<&str>, max_chars: usize) -> Option<String> {
    let text: String = value?
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(max_chars)
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Stores what PayPal reported for a completed capture: amount, capture ID, and payer.
async fn record_capture(state: &AppState, order_id: &str, order: &Value) {
    state
        .orders
        .lock()
        .await
        .insert(order_id.to_owned(), OrderState::Captured);
    let capture = &order["purchase_units"][0]["payments"]["captures"][0];
    let amount = copilot::money_value(&capture["amount"]);
    let payer_name = plain(
        Some(
            &[
                order["payer"]["name"]["given_name"].as_str(),
                order["payer"]["name"]["surname"].as_str(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" "),
        ),
        80,
    );
    let payer_email = plain(order["payer"]["email_address"].as_str(), 254);
    let mut ledger = state.ledger.lock().await;
    let mut detail = format!("Order {order_id}");
    if let Some(record) = ledger.order_mut(order_id) {
        record.state = "captured".to_owned();
        record.captured_at = Some(copilot::now_seconds());
        record.capture_id = capture["id"]
            .as_str()
            .filter(|id| valid_order_id(id))
            .map(str::to_owned);
        match amount {
            Some((cents, currency)) if cents > 0 => {
                record.captured_cents = cents;
                record.currency = currency;
            }
            _ => record.captured_cents = record.amount_cents,
        }
        record.payer_email = payer_email;
        record.payer_name = payer_name;
        let payer = record
            .payer_name
            .as_deref()
            .or(record.payer_email.as_deref())
            .map(|payer| format!(" from {payer}"))
            .unwrap_or_default();
        detail = format!(
            "{} · {}{payer}",
            copilot::usd(record.captured_cents),
            record.description
        );
    }
    ledger.event("payment", "Payment captured", detail);
    ledger.save();
}

/// Adds a refund PayPal accepted to the order's record.
pub(crate) async fn record_refund(
    state: &AppState,
    order_id: &str,
    refund_id: Option<&str>,
    cents: i64,
) {
    let refund_id = refund_id
        .filter(|id| valid_order_id(id))
        .map(str::to_owned);
    let mut ledger = state.ledger.lock().await;
    let mut detail = format!("{} on order {order_id}", copilot::usd(cents));
    if let Some(order) = ledger.order_mut(order_id) {
        // A retried request with the same PayPal-Request-Id returns the same refund; count it once.
        if refund_id.is_some() && order.refunds.iter().any(|refund| refund.id == refund_id) {
            return;
        }
        order.refunds.push(ledger::RefundRecord {
            id: refund_id,
            at: copilot::now_seconds(),
            cents,
        });
        detail = format!("{} · {} · order {order_id}", copilot::usd(cents), order.description);
    }
    ledger.event("refund", "Refund issued", detail);
    ledger.save();
}

/// Creates a USD order the buyer approves on PayPal. Returns the order ID and approval link.
async fn create_paypal_checkout_order(
    state: &AppState,
    amount_cents: i64,
    description: &str,
    request_id: &str,
    source: &str,
) -> Result<(String, String), ApiError> {
    let brand = state.ledger.lock().await.data.profile.business_name.clone();
    let body = paypal_order_body(
        &state.app_base_url,
        &copilot::decimal(amount_cents),
        description,
        &brand,
    );
    let result =
        paypal_api_request_with_id(state, Method::POST, "/v2/checkout/orders", body, request_id)
            .await?;

    let order_id = match result["id"].as_str() {
        Some(order_id) if valid_order_id(order_id) => order_id.to_owned(),
        _ => {
            return Err(ApiError::new(
                "PayPal did not return a usable Sandbox order ID.",
            ));
        }
    };
    let approval_url = result["links"].as_array().and_then(|links| {
        links.iter().find_map(|link| {
            let relation = link["rel"].as_str()?;
            let href = link["href"].as_str()?;
            (matches!(relation, "approve" | "payer-action") && valid_approval_url(href))
                .then(|| href.to_owned())
        })
    });
    let Some(approval_url) = approval_url else {
        return Err(ApiError::new(
            "PayPal did not return a usable Sandbox checkout link.",
        ));
    };

    record_new_order(
        state,
        new_order_record(
            &order_id,
            source,
            description,
            amount_cents,
            VerificationMode::PayPalApproval,
            Some(approval_url.clone()),
        ),
    )
    .await;
    Ok((order_id, approval_url))
}

fn new_order_record(
    order_id: &str,
    source: &str,
    description: &str,
    amount_cents: i64,
    verification: VerificationMode,
    approval_url: Option<String>,
) -> ledger::OrderRecord {
    ledger::OrderRecord {
        id: order_id.to_owned(),
        created_at: copilot::now_seconds(),
        source: source.to_owned(),
        description: description.to_owned(),
        amount_cents,
        currency: "USD".to_owned(),
        state: "created".to_owned(),
        verification: verification_name(verification).to_owned(),
        customer: None,
        approval_url,
        captured_at: None,
        capture_id: None,
        captured_cents: 0,
        payer_email: None,
        payer_name: None,
        refunds: Vec::new(),
    }
}

async fn create_order(State(state): State<AppState>) -> Response {
    let request_id = Uuid::new_v4().to_string();
    match create_paypal_checkout_order(
        &state,
        1_000,
        "Boardroom Sandbox checkout test",
        &request_id,
        "checkout",
    )
    .await
    {
        Ok((order_id, approval_url)) => (
            StatusCode::CREATED,
            Json(json!({
                "orderId": order_id,
                "approvalUrl": approval_url,
            })),
        )
            .into_response(),
        Err(error) => api_error(error),
    }
}

async fn card_checkout_config(State(state): State<AppState>) -> Response {
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({ "clientId": state.client_id })),
    )
        .into_response()
}

fn card_order_body(base_url: &str, scenario: CardScenario) -> Value {
    let verification = match scenario {
        CardScenario::ThreeDs => "SCA_ALWAYS",
        CardScenario::Success | CardScenario::Decline => "SCA_WHEN_REQUIRED",
    };

    json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "description": "Boardroom Sandbox checkout test",
            "amount": { "currency_code": "USD", "value": "10.00" }
        }],
        "payment_source": {
            "card": {
                "attributes": { "verification": { "method": verification } },
                "experience_context": {
                    "shipping_preference": "NO_SHIPPING",
                    "return_url": format!("{base_url}/checkout/complete"),
                    "cancel_url": format!("{base_url}/checkout/cancel")
                }
            }
        }
    })
}

async fn create_card_order(
    State(state): State<AppState>,
    Json(request): Json<CardOrderRequest>,
) -> Response {
    let body = card_order_body(&state.app_base_url, request.scenario);
    let result = match paypal_api_request(&state, Method::POST, "/v2/checkout/orders", body).await {
        Ok(result) => result,
        Err(error) => return api_error(error),
    };
    let order_id = match result["id"].as_str() {
        Some(order_id) if valid_order_id(order_id) => order_id.to_owned(),
        _ => {
            return api_error(ApiError::new(
                "PayPal did not return a usable Sandbox order ID.",
            ));
        }
    };

    let verification = match request.scenario {
        CardScenario::ThreeDs => VerificationMode::CardAlways,
        CardScenario::Success | CardScenario::Decline => VerificationMode::CardWhenRequired,
    };
    record_new_order(
        &state,
        new_order_record(
            &order_id,
            "card",
            "Boardroom Sandbox checkout test",
            1_000,
            verification,
            None,
        ),
    )
    .await;
    (StatusCode::CREATED, Json(json!({ "orderId": order_id }))).into_response()
}

async fn capture_order(State(state): State<AppState>, Path(order_id): Path<String>) -> Response {
    if !valid_order_id(&order_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid Sandbox order ID." })),
        )
            .into_response();
    }

    let verification = {
        let mut orders = state.orders.lock().await;
        let verification = match orders.get(&order_id) {
            Some(OrderState::Created { verification }) => *verification,
            _ => {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": "This order was not created by this app or has already been processed.",
                    })),
                )
                    .into_response();
            }
        };
        orders.insert(order_id.clone(), OrderState::CapturePending);
        verification
    };

    if verification != VerificationMode::PayPalApproval {
        let order = match paypal_api_request(
            &state,
            Method::GET,
            &format!("/v2/checkout/orders/{order_id}"),
            json!(null),
        )
        .await
        {
            Ok(order) => order,
            Err(error) => {
                mark_order(&state, &order_id, OrderState::CaptureUnknown).await;
                return api_error(error);
            }
        };
        if !confirmed_card_order(&order, &order_id, verification) {
            mark_order(&state, &order_id, OrderState::CaptureUnknown).await;
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "Card approval or required 3D Secure verification could not be confirmed. Capture was not requested; check Sandbox activity.",
                })),
            )
                .into_response();
        }
    }

    let result = match paypal_api_request(
        &state,
        Method::POST,
        &format!("/v2/checkout/orders/{order_id}/capture"),
        json!({}),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            mark_order(&state, &order_id, OrderState::CaptureUnknown).await;
            return api_error(error);
        }
    };

    let response_order_id = result["id"].as_str();
    let status = result["status"].as_str();
    if response_order_id != Some(order_id.as_str()) || status.is_none() {
        mark_order(&state, &order_id, OrderState::CaptureUnknown).await;
        return api_error(ApiError::new(
            "PayPal returned an incomplete capture response. Verify Sandbox activity.",
        ));
    }
    let status = status.unwrap_or_default().to_owned();
    if status == "COMPLETED" {
        // The capture response may omit details; read the order back if so.
        let details = if result["purchase_units"][0]["payments"]["captures"][0].is_object() {
            result
        } else {
            paypal_api_request(
                &state,
                Method::GET,
                &format!("/v2/checkout/orders/{order_id}"),
                Value::Null,
            )
            .await
            .unwrap_or(result)
        };
        record_capture(&state, &order_id, &details).await;
    } else {
        mark_order(&state, &order_id, OrderState::CaptureUnknown).await;
    }

    Json(json!({
        "orderId": order_id,
        "status": status,
    }))
    .into_response()
}

fn confirmed_card_order(order: &Value, order_id: &str, verification: VerificationMode) -> bool {
    if order["id"].as_str() != Some(order_id)
        || order["status"].as_str() != Some("APPROVED")
        || !order["payment_source"]["card"].is_object()
    {
        return false;
    }

    let authentication = &order["payment_source"]["card"]["authentication_result"];
    match verification {
        VerificationMode::CardAlways => {
            authentication["liability_shift"].as_str() == Some("POSSIBLE")
        }
        VerificationMode::CardWhenRequired if authentication.is_null() => true,
        VerificationMode::CardWhenRequired => {
            authentication["liability_shift"].as_str() == Some("POSSIBLE")
        }
        VerificationMode::PayPalApproval => false,
    }
}

async fn checkout_page(State(state): State<AppState>) -> Response {
    match tokio::fs::read_to_string(state.public_directory.join("index.html")).await {
        Ok(contents) => Html(contents).into_response(),
        Err(error) => {
            eprintln!("Could not read the dashboard page: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load the dashboard page.",
            )
                .into_response()
        }
    }
}

fn load_state() -> Result<AppState, String> {
    if env::var("NODE_ENV").as_deref() == Ok("production") {
        return Err("This demo server only supports the PayPal Sandbox.".to_owned());
    }

    let port = env::var("PORT")
        .unwrap_or_else(|_| "3000".to_owned())
        .parse::<u16>()
        .map_err(|_| "PORT must be a valid TCP port.".to_owned())?;
    if port == 0 {
        return Err("PORT must be a valid TCP port.".to_owned());
    }

    let (negative_testing_enabled, negative_test_type) = negative_testing_config()?;
    let public_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Could not initialize the HTTP client: {error}"))?;
    let data_directory = env::var("DATA_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"));
    let ledger = ledger::Store::open(data_directory.join("ledger.json"))?;
    let (orders, order_log, invoices) = hydrate(&ledger.data);

    Ok(AppState {
        client,
        port,
        client_id: required_credential("PAYPAL_CLIENT_ID")?,
        client_secret: required_credential("PAYPAL_CLIENT_SECRET")?,
        app_base_url: configured_base_url()?,
        public_directory,
        negative_testing_enabled,
        negative_test_type,
        paypal_api: PAYPAL_API.to_owned(),
        orders: Arc::new(Mutex::new(orders)),
        order_log: Arc::new(Mutex::new(order_log)),
        invoices: Arc::new(Mutex::new(invoices)),
        token_cache: Arc::new(Mutex::new(None)),
        ai: llm::config_from_env()?.map(Arc::new),
        copilot: Arc::new(Mutex::new(copilot::ActionBook::default())),
        ledger: Arc::new(Mutex::new(ledger)),
        brief_busy: Arc::new(Mutex::new(())),
    })
}

fn load_dotenv() -> Result<(), String> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not load .env: {error}")),
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = load_dotenv() {
        eprintln!("{error}");
        std::process::exit(1);
    }

    let state = match load_state() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let port = state.port;
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Could not bind to port {port}: {error}");
            std::process::exit(1);
        }
    };
    let public_directory = state.public_directory.clone();
    let ai_label = state.ai.as_ref().map(|ai| ai.label());
    let ledger_path = state
        .ledger
        .lock()
        .await
        .path()
        .map(|path| path.display().to_string());
    let app = Router::new()
        .route("/api/checkout/orders", post(create_order))
        .route("/api/copilot/status", get(copilot::status))
        .route("/api/copilot/chat", post(copilot::chat))
        .route(
            "/api/copilot/actions/{action_id}/approve",
            post(copilot::approve),
        )
        .route(
            "/api/copilot/actions/{action_id}/dismiss",
            post(copilot::dismiss),
        )
        .route("/api/sandbox/transactions", get(sandbox_transactions))
        .route(
            "/api/sandbox/invoices/{invoice_id}/send",
            post(send_sandbox_invoice),
        )
        .route("/api/status", get(copilot::status))
        .route("/api/dashboard", get(workspace::dashboard))
        .route("/api/activity", get(workspace::activity))
        .route("/api/wallet", get(workspace::wallet))
        .route(
            "/api/profile",
            get(workspace::profile).post(workspace::update_profile),
        )
        .route("/api/payments/request", post(workspace::request_payment))
        .route("/api/orders/{order_id}/refund", post(workspace::refund_order))
        .route("/api/payouts", post(workspace::create_payout))
        .route("/api/payouts/{batch_id}", get(workspace::payout_status))
        .route(
            "/api/invoices",
            get(workspace::list_invoices).post(workspace::create_invoice),
        )
        .route(
            "/api/invoices/{invoice_id}/{action}",
            post(workspace::invoice_action),
        )
        .route(
            "/api/boardroom/brief",
            get(boardroom::latest).post(boardroom::generate),
        )
        .route("/api/checkout/card/config", get(card_checkout_config))
        .route("/api/checkout/card/orders", post(create_card_order))
        .route(
            "/api/checkout/orders/{order_id}/capture",
            post(capture_order),
        )
        .route("/checkout/complete", get(checkout_page))
        .route("/checkout/cancel", get(checkout_page))
        .fallback_service(ServeDir::new(public_directory).append_index_html_on_directories(true))
        .with_state(state);

    println!("Boardroom Rust Sandbox demo listening at http://localhost:{port}");
    if let Some(path) = ledger_path {
        println!("Activity record: {path}");
    }
    match ai_label {
        Some(label) => println!("Boardroom Copilot AI: {label}"),
        None => println!(
            "Boardroom Copilot AI is off. Set XAI_API_KEY or OPENAI_API_KEY in .env to enable it."
        ),
    }
    if let Err(error) = axum::serve(listener, app).await {
        eprintln!("The HTTP server stopped unexpectedly: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CardOrderRequest, CardScenario, InvoiceState, OrderState, VerificationMode,
        card_order_body, confirmed_card_order, hydrate, invoice_id_from_create_response, ledger,
        new_order_record, paypal_error_message, paypal_issue, paypal_order_body,
        valid_approval_url, valid_invoice_id, valid_order_id, valid_test_recipient,
        valid_timestamp,
    };
    use serde_json::json;

    #[test]
    fn paypal_order_uses_experience_context_not_deprecated_application_context() {
        let body = paypal_order_body(
            "http://localhost:3000",
            "45.00",
            "Everyday Tote",
            "Avery's Bags",
        );
        assert!(body.get("application_context").is_none());
        assert_eq!(body["intent"], "CAPTURE");
        assert_eq!(body["purchase_units"][0]["amount"]["value"], "45.00");
        assert_eq!(body["purchase_units"][0]["description"], "Everyday Tote");
        let context = &body["payment_source"]["paypal"]["experience_context"];
        assert_eq!(context["brand_name"], "Avery's Bags");
        assert_eq!(context["user_action"], "PAY_NOW");
        assert_eq!(
            context["return_url"],
            "http://localhost:3000/checkout/complete"
        );
        assert_eq!(
            context["cancel_url"],
            "http://localhost:3000/checkout/cancel"
        );
    }

    #[test]
    fn invoice_id_is_read_only_from_trusted_links() {
        let sandbox = "https://api-m.sandbox.paypal.com";
        let ok = json!({ "href": "https://api-m.sandbox.paypal.com/v2/invoicing/invoices/INV2-AAAA-BBBB-CCCC-DDDD" });
        assert_eq!(
            invoice_id_from_create_response(&ok, sandbox).as_deref(),
            Some("INV2-AAAA-BBBB-CCCC-DDDD")
        );
        let foreign =
            json!({ "href": "https://evil.example.com/v2/invoicing/invoices/INV2-AAAA-BBBB" });
        assert!(invoice_id_from_create_response(&foreign, sandbox).is_none());
        let mock = json!({ "href": "http://127.0.0.1:9/v2/invoicing/invoices/INV2-MOCK-0001" });
        assert_eq!(
            invoice_id_from_create_response(&mock, "http://127.0.0.1:9").as_deref(),
            Some("INV2-MOCK-0001")
        );
        assert!(invoice_id_from_create_response(&mock, sandbox).is_none());
    }

    #[test]
    fn missing_sandbox_permission_explains_how_to_enable_it() {
        let denied =
            json!({ "name": "NOT_AUTHORIZED", "details": [{ "issue": "NOT_AUTHORIZED" }] });
        assert!(paypal_error_message(&denied).contains("Developer Dashboard"));
        assert_eq!(paypal_issue(&denied).as_deref(), Some("NOT_AUTHORIZED"));
        assert_eq!(
            paypal_issue(&json!({ "name": "RESOURCE_NOT_FOUND" })).as_deref(),
            Some("RESOURCE_NOT_FOUND")
        );
        assert!(paypal_issue(&json!(null)).is_none());
    }

    #[test]
    fn validates_sandbox_order_ids() {
        assert!(valid_order_id("5O190127TN364715T"));
        assert!(!valid_order_id("too-short"));
        assert!(!valid_order_id("5O190127TN364715T/extra"));
    }

    #[test]
    fn only_example_com_test_recipients_are_accepted() {
        assert!(valid_test_recipient("demo.customer@example.com"));
        assert!(valid_test_recipient("sb-47abc12345678@personal.example.com"));
        assert!(valid_test_recipient("sb-1x2y3z@business.example.com"));
        assert!(!valid_test_recipient("customer@real-domain.com"));
        assert!(!valid_test_recipient("customer@notexample.com"));
        assert!(!valid_test_recipient("customer@example.com.evil.com"));
        assert!(!valid_test_recipient("customer@-bad.example.com"));
        assert!(!valid_test_recipient("customer@.example.com"));
        assert!(!valid_test_recipient("customer@Personal.example.com"));
        assert!(!valid_test_recipient("a@b@example.com"));
        assert!(!valid_test_recipient(".bad@example.com"));
        assert!(!valid_test_recipient("bad..name@example.com"));
        assert!(valid_invoice_id("INV2-ABCD-EFGH-1234"));
        assert!(!valid_invoice_id("INV2-ABCD/anything"));
    }

    #[test]
    fn restart_restores_order_and_invoice_state_without_retrying_captures() {
        let mut data = ledger::Data::default();
        data.orders.push(new_order_record(
            "ORDERCREATED001",
            "card",
            "Test",
            1_000,
            VerificationMode::CardAlways,
            None,
        ));
        let mut captured = new_order_record(
            "ORDERCAPTURED01",
            "checkout",
            "Test",
            1_000,
            VerificationMode::PayPalApproval,
            None,
        );
        captured.state = "captured".to_owned();
        data.orders.push(captured);
        let mut in_flight = captured_like("ORDERPENDING001");
        in_flight.state = "capture_pending".to_owned();
        data.orders.push(in_flight);
        for (id, status) in [
            ("INV2-DRAFT", "draft"),
            ("INV2-SENT", "sent"),
            ("INV2-HALF", "send_pending"),
        ] {
            data.invoices.push(ledger::InvoiceRecord {
                id: id.to_owned(),
                created_at: 0,
                recipient: "a@example.com".to_owned(),
                amount_cents: 100,
                status: status.to_owned(),
            });
        }

        let (orders, log, invoices) = hydrate(&data);
        assert_eq!(
            orders["ORDERCREATED001"],
            OrderState::Created {
                verification: VerificationMode::CardAlways
            }
        );
        assert_eq!(orders["ORDERCAPTURED01"], OrderState::Captured);
        assert_eq!(orders["ORDERPENDING001"], OrderState::CaptureUnknown);
        assert_eq!(log, ["ORDERCREATED001", "ORDERCAPTURED01", "ORDERPENDING001"]);
        assert_eq!(invoices["INV2-DRAFT"], InvoiceState::Draft);
        assert_eq!(invoices["INV2-SENT"], InvoiceState::Sent);
        assert_eq!(invoices["INV2-HALF"], InvoiceState::SendUnknown);
    }

    fn captured_like(id: &str) -> ledger::OrderRecord {
        new_order_record(id, "checkout", "Test", 1_000, VerificationMode::PayPalApproval, None)
    }

    #[test]
    fn payout_and_permission_errors_explain_what_to_do() {
        assert!(
            paypal_error_message(&json!({ "name": "INSUFFICIENT_FUNDS" }))
                .contains("insufficient funds")
        );
        assert!(
            paypal_error_message(&json!({ "name": "AUTHORIZATION_ERROR" }))
                .contains("Developer Dashboard")
        );
    }

    #[test]
    fn validates_transaction_timestamp_shape() {
        assert!(valid_timestamp("2026-11-03T10:15:00Z"));
        assert!(!valid_timestamp("2026-11-03T10:15:00Z&fields=all"));
        assert!(!valid_timestamp("2026-11-03"));
    }

    #[test]
    fn accepts_only_https_paypal_approval_links() {
        assert!(valid_approval_url(
            "https://www.sandbox.paypal.com/checkoutnow?token=5O190127TN364715T"
        ));
        assert!(!valid_approval_url(
            "http://www.sandbox.paypal.com/checkoutnow"
        ));
        assert!(!valid_approval_url(
            "https://example.com/checkoutnow?token=5O190127TN364715T"
        ));
    }

    #[test]
    fn card_order_uses_fixed_amount_and_requested_verification() {
        for (scenario, expected_method) in [
            (CardScenario::Success, "SCA_WHEN_REQUIRED"),
            (CardScenario::Decline, "SCA_WHEN_REQUIRED"),
            (CardScenario::ThreeDs, "SCA_ALWAYS"),
        ] {
            let body = card_order_body("http://localhost:3000", scenario);
            assert_eq!(body["purchase_units"][0]["amount"]["value"], "10.00");
            assert_eq!(body["purchase_units"][0]["amount"]["currency_code"], "USD");
            assert_eq!(
                body["payment_source"]["card"]["attributes"]["verification"]["method"],
                expected_method
            );
            assert_eq!(
                body["payment_source"]["card"]["experience_context"]["return_url"],
                "http://localhost:3000/checkout/complete"
            );
        }
    }

    #[test]
    fn card_order_rejects_unknown_scenarios_and_extra_fields() {
        assert!(serde_json::from_str::<CardOrderRequest>(r#"{"scenario":"three_ds"}"#).is_ok());
        assert!(serde_json::from_str::<CardOrderRequest>(r#"{"scenario":"other"}"#).is_err());
        assert!(
            serde_json::from_str::<CardOrderRequest>(r#"{"scenario":"success","amount":"0.01"}"#)
                .is_err()
        );
    }

    #[test]
    fn three_ds_capture_requires_matching_approved_order_and_possible_shift() {
        let mut order = json!({
            "id": "5O190127TN364715T",
            "status": "APPROVED",
            "payment_source": { "card": { "authentication_result": { "liability_shift": "POSSIBLE" } } }
        });
        assert!(confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardAlways
        ));
        assert!(!confirmed_card_order(
            &order,
            "DIFFERENTORDER",
            VerificationMode::CardAlways
        ));
        order["payment_source"]["card"]["authentication_result"]["liability_shift"] = json!("NO");
        assert!(!confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardAlways
        ));
        assert!(!confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardWhenRequired
        ));
        order["payment_source"]["card"]["authentication_result"]["liability_shift"] =
            json!("UNKNOWN");
        assert!(!confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardAlways
        ));
        order["payment_source"]["card"]["authentication_result"] = json!({});
        assert!(!confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardAlways
        ));
        order["payment_source"]["card"]["authentication_result"] = json!(null);
        assert!(confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardWhenRequired
        ));
        assert!(!confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardAlways
        ));
        order["payment_source"]["card"] = json!(null);
        assert!(!confirmed_card_order(
            &order,
            "5O190127TN364715T",
            VerificationMode::CardWhenRequired
        ));
    }

    #[test]
    fn sandbox_error_message_only_surfaces_known_decline() {
        let decline = json!({ "details": [{ "issue": "INSTRUMENT_DECLINED" }] });
        assert!(paypal_error_message(&decline).contains("declined"));
        let other = json!({ "details": [{ "issue": "SOME_OTHER_ERROR" }] });
        assert!(paypal_error_message(&other).contains("server logs"));
    }
}
