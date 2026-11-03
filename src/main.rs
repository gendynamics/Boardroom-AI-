use std::{collections::HashMap, env, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{Method, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use reqwest::{Client, Url};
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
    client_id: String,
    client_secret: String,
    app_base_url: String,
    public_directory: PathBuf,
    negative_testing_enabled: bool,
    negative_test_type: String,
    orders: Arc<Mutex<HashMap<String, OrderState>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OrderState {
    Created,
    CapturePending,
    Captured,
    CaptureUnknown,
}

struct ApiError {
    message: &'static str,
}

impl ApiError {
    fn new(message: &'static str) -> Self {
        Self { message }
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

async fn paypal_access_token(state: &AppState) -> Result<String, ApiError> {
    let response = state
        .client
        .post(format!("{PAYPAL_API}/v1/oauth2/token"))
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
    payload["access_token"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ApiError::new("PayPal Sandbox did not return an access token."))
}

async fn paypal_api_request(
    state: &AppState,
    method: Method,
    path: &str,
    body: Value,
) -> Result<Value, ApiError> {
    let access_token = paypal_access_token(state).await?;
    let request_id = Uuid::new_v4().to_string();
    let mut request = state
        .client
        .request(method, format!("{PAYPAL_API}{path}"))
        .bearer_auth(access_token)
        .header("PayPal-Request-Id", request_id)
        .json(&body);

    if state.negative_testing_enabled {
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
        eprintln!("PayPal Sandbox API returned HTTP {status} for {path}.");
        return Err(ApiError::new(
            "PayPal Sandbox could not complete the request. Check the server logs.",
        ));
    }

    response
        .json()
        .await
        .map_err(|_| ApiError::new("PayPal Sandbox returned an invalid API response."))
}

async fn create_order(State(state): State<AppState>) -> Response {
    let body = json!({
        "intent": "CAPTURE",
        "application_context": {
            "brand_name": "The Boardroom Demo",
            "user_action": "PAY_NOW",
            "return_url": format!("{}/checkout/complete", state.app_base_url),
            "cancel_url": format!("{}/checkout/cancel", state.app_base_url),
        },
        "purchase_units": [{
            "description": "Boardroom Sandbox checkout test",
            "amount": {
                "currency_code": "USD",
                "value": "10.00",
            },
        }],
    });

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
    let approval_url = result["links"].as_array().and_then(|links| {
        links.iter().find_map(|link| {
            let relation = link["rel"].as_str()?;
            let href = link["href"].as_str()?;
            (matches!(relation, "approve" | "payer-action") && valid_approval_url(href))
                .then(|| href.to_owned())
        })
    });
    let Some(approval_url) = approval_url else {
        return api_error(ApiError::new(
            "PayPal did not return a usable Sandbox checkout link.",
        ));
    };

    state
        .orders
        .lock()
        .await
        .insert(order_id.clone(), OrderState::Created);

    (
        StatusCode::CREATED,
        Json(json!({
            "orderId": order_id,
            "approvalUrl": approval_url,
        })),
    )
        .into_response()
}

async fn capture_order(State(state): State<AppState>, Path(order_id): Path<String>) -> Response {
    if !valid_order_id(&order_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid Sandbox order ID." })),
        )
            .into_response();
    }

    {
        let mut orders = state.orders.lock().await;
        if orders.get(&order_id) != Some(&OrderState::Created) {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "This order was not created by this running demo or has already been processed.",
                })),
            )
                .into_response();
        }
        orders.insert(order_id.clone(), OrderState::CapturePending);
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
            state
                .orders
                .lock()
                .await
                .insert(order_id, OrderState::CaptureUnknown);
            return api_error(error);
        }
    };

    let response_order_id = result["id"].as_str();
    let status = result["status"].as_str();
    if response_order_id != Some(order_id.as_str()) || status.is_none() {
        state
            .orders
            .lock()
            .await
            .insert(order_id, OrderState::CaptureUnknown);
        return api_error(ApiError::new(
            "PayPal returned an incomplete capture response. Verify Sandbox activity.",
        ));
    }
    let status = status.unwrap_or_default();
    state.orders.lock().await.insert(
        order_id.clone(),
        if status == "COMPLETED" {
            OrderState::Captured
        } else {
            OrderState::CaptureUnknown
        },
    );

    Json(json!({
        "orderId": order_id,
        "status": status,
    }))
    .into_response()
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

    Ok(AppState {
        client,
        port,
        client_id: required_credential("PAYPAL_CLIENT_ID")?,
        client_secret: required_credential("PAYPAL_CLIENT_SECRET")?,
        app_base_url: configured_base_url()?,
        public_directory,
        negative_testing_enabled,
        negative_test_type,
        orders: Arc::new(Mutex::new(HashMap::new())),
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
    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Could not bind to port {port}: {error}");
            std::process::exit(1);
        }
    };
    let public_directory = state.public_directory.clone();
    let app = Router::new()
        .route("/api/checkout/orders", post(create_order))
        .route(
            "/api/checkout/orders/{order_id}/capture",
            post(capture_order),
        )
        .route("/checkout/complete", get(checkout_page))
        .route("/checkout/cancel", get(checkout_page))
        .fallback_service(ServeDir::new(public_directory).append_index_html_on_directories(true))
        .with_state(state);

    println!("Boardroom Rust Sandbox demo listening at http://localhost:{port}");
    if let Err(error) = axum::serve(listener, app).await {
        eprintln!("The HTTP server stopped unexpectedly: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{valid_approval_url, valid_order_id};

    #[test]
    fn validates_sandbox_order_ids() {
        assert!(valid_order_id("5O190127TN364715T"));
        assert!(!valid_order_id("too-short"));
        assert!(!valid_order_id("5O190127TN364715T/extra"));
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
}
