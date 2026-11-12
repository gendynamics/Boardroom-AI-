//! The Boardroom brief: the AI reads dashboard figures computed from the merchant's real
//! Sandbox activity and writes one recommendation from four advisor perspectives, plus a first
//! step the merchant can hand to Boardroom Copilot. Copilot still needs approval for anything
//! it creates, so the brief itself never touches PayPal.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Map, Value, json};

use crate::{
    AppState,
    copilot::{Capabilities, message_text, now_seconds, utc_timestamp},
    llm, paypal_token, same_origin, workspace,
};

pub const ROLES: [&str; 4] = ["Strategist", "Marketer", "CFO", "Growth Lead"];

fn reply(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn error(status: StatusCode, message: &str) -> Response {
    reply(status, json!({ "error": message }))
}

/// Cleans model text: control characters become spaces, and long text is cut at a word.
fn fit(value: &Value, max_chars: usize) -> Option<String> {
    let text: String = value
        .as_str()?
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= max_chars {
        return Some(text);
    }
    let cut: String = text.chars().take(max_chars - 1).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > max_chars / 2 => cut[..space].to_owned(),
        _ => cut,
    };
    Some(format!("{}…", cut.trim_end_matches([',', '.', ';', ':', ' '])))
}

/// Parses and validates the model's JSON brief. Any missing field rejects the whole brief.
pub(crate) fn parse_brief(text: &str) -> Result<Value, &'static str> {
    const UNUSABLE: &str = "The AI returned an unusable brief. Try again.";
    let start = text.find('{').ok_or(UNUSABLE)?;
    let end = text.rfind('}').ok_or(UNUSABLE)?;
    if end <= start {
        return Err(UNUSABLE);
    }
    let raw: Value = serde_json::from_str(&text[start..=end]).map_err(|_| UNUSABLE)?;
    let field = |value: &Value, max: usize| fit(value, max).ok_or(UNUSABLE);
    let experiment = &raw["experiment"];
    let mut perspectives = Vec::with_capacity(ROLES.len());
    for role in ROLES {
        let take = raw["perspectives"]
            .as_array()
            .ok_or(UNUSABLE)?
            .iter()
            .find(|item| {
                item["role"]
                    .as_str()
                    .is_some_and(|name| name.trim().eq_ignore_ascii_case(role))
            })
            .ok_or(UNUSABLE)?;
        perspectives.push(json!({ "role": role, "take": field(&take["take"], 260)? }));
    }
    let mut brief = Map::new();
    brief.insert("headline".into(), json!(field(&raw["headline"], 90)?));
    brief.insert("summary".into(), json!(field(&raw["summary"], 500)?));
    brief.insert(
        "experiment".into(),
        json!({
            "title": field(&experiment["title"], 90)?,
            "audience": field(&experiment["audience"], 160)?,
            "offer": field(&experiment["offer"], 160)?,
            "duration": field(&experiment["duration"], 60)?,
            "guardrail": field(&experiment["guardrail"], 160)?,
            "successMeasure": field(&experiment["success_measure"], 160)?
        }),
    );
    brief.insert("perspectives".into(), Value::Array(perspectives));
    brief.insert("verdict".into(), json!(field(&raw["verdict"], 400)?));
    brief.insert(
        "copilotRequest".into(),
        json!(field(&raw["copilot_request"], 300)?),
    );
    Ok(Value::Object(brief))
}

fn system_prompt(capabilities: Capabilities) -> String {
    let mut can = vec![
        "create PayPal checkout links (payment links) for a stated amount and item",
        "check whether recent checkout orders were paid",
        "refund captured orders",
    ];
    if capabilities.invoicing {
        can.push("draft and send itemized PayPal invoices to test addresses");
    }
    if capabilities.transaction_search {
        can.push("summarize account-wide PayPal transaction history");
    }
    format!(
        "You are the Boardroom: four advisors (Strategist, Marketer, CFO, Growth Lead) for a small business that sells with PayPal. The figures you receive come from the merchant's PayPal Sandbox activity through this app (test money).

Rules:
- Use only the figures provided. Never invent customers, sales, conversion rates, or industry benchmarks.
- If there is little or no data, say so plainly and recommend a practical first step that produces real data, such as sharing a payment link.
- Recommend one small, reversible experiment. Its first step must be something Boardroom Copilot can do: {}. Anything else (posting on social media, emailing customers) is for the merchant to do themselves; say so.
- No definitive financial, tax, or legal advice. Plain text inside the JSON strings, no Markdown.

Respond with only one JSON object, no code fences:
{{\"headline\": \"under 12 words\", \"summary\": \"2-3 sentences citing the figures\", \"experiment\": {{\"title\": \"...\", \"audience\": \"...\", \"offer\": \"...\", \"duration\": \"...\", \"guardrail\": \"a cap or stop condition\", \"success_measure\": \"...\"}}, \"perspectives\": [{{\"role\": \"Strategist\", \"take\": \"under 40 words\"}}, {{\"role\": \"Marketer\", \"take\": \"...\"}}, {{\"role\": \"CFO\", \"take\": \"...\"}}, {{\"role\": \"Growth Lead\", \"take\": \"...\"}}], \"verdict\": \"1-2 sentences\", \"copilot_request\": \"one instruction the merchant can send to Boardroom Copilot for the first step, with a concrete amount and item\"}}",
        can.join("; ")
    )
}

/// The figures the advisors see: the 30-day dashboard plus all-time totals.
fn facts(data: &crate::ledger::Data, now: i64) -> (Value, usize) {
    let summary = workspace::summarize(data, now, 30);
    let balance = workspace::boardroom_balance(data);
    let payments = data.orders.iter().filter(|order| order.is_paid()).count();
    let mut invoice_statuses = std::collections::BTreeMap::<String, u32>::new();
    for invoice in &data.invoices {
        *invoice_statuses.entry(invoice.status.clone()).or_default() += 1;
    }
    let facts = json!({
        "business_name": data.profile.business_name,
        "today": &utc_timestamp(now)[..10],
        "source": summary["source"],
        "last_30_days": {
            "gross_sales": summary["gross"]["text"],
            "gross_change_pct_vs_previous_30_days": summary["gross"]["changePct"],
            "paid_orders": summary["orders"]["count"],
            "paid_orders_previous_30_days": summary["orders"]["previous"],
            "average_order_value": summary["averageOrder"]["text"],
            "paying_customers": summary["repeatCustomers"]["customers"],
            "repeat_customers": summary["repeatCustomers"]["repeat"],
            "refunds": summary["refunds"]["text"],
            "refunds_pct_of_gross": summary["refunds"]["pctOfGross"],
            "net_sales": summary["net"]["text"],
            "top_items": summary["topItems"],
            "unpaid_payment_links": summary["unpaid"]
        },
        "all_time": {
            "paid_orders": payments,
            "captured": balance["captured"],
            "refunded": balance["refunded"],
            "paid_out": balance["paidOut"],
            "invoices_by_status": invoice_statuses
        }
    });
    (facts, payments)
}

pub async fn latest(State(state): State<AppState>) -> Response {
    let brief = state.ledger.lock().await.data.brief.clone();
    reply(
        StatusCode::OK,
        json!({ "brief": brief, "aiConfigured": state.ai.is_some() }),
    )
}

pub async fn generate(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !same_origin(&headers, &state) {
        return error(
            StatusCode::FORBIDDEN,
            "Open the app from its configured origin (APP_BASE_URL).",
        );
    }
    let Some(ai) = state.ai.clone() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The AI is off. Add XAI_API_KEY (or OPENAI_API_KEY) to .env and restart the server.",
        );
    };
    let Ok(_running) = state.brief_busy.try_lock() else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "A brief is already being written. Wait for it to finish.",
        );
    };
    let capabilities = paypal_token(&state)
        .await
        .map(|token| Capabilities::from_scope(&token.scope))
        .unwrap_or_default();
    let now = now_seconds();
    let (figures, payments) = facts(&state.ledger.lock().await.data, now);
    let messages = [
        json!({ "role": "system", "content": system_prompt(capabilities) }),
        json!({ "role": "user", "content": format!("Figures (JSON):\n{figures}") }),
    ];
    let message = match llm::chat(&state.client, &ai, &messages, &[]).await {
        Ok(message) => message,
        Err(message) => return error(StatusCode::BAD_GATEWAY, message),
    };
    let mut brief = match parse_brief(&message_text(&message)) {
        Ok(brief) => brief,
        Err(message) => return error(StatusCode::BAD_GATEWAY, message),
    };
    brief["generatedAt"] = json!(utc_timestamp(now));
    brief["model"] = json!(ai.label());
    brief["basis"] = json!({ "days": 30, "paidOrders": payments, "source": figures["source"] });

    let mut ledger = state.ledger.lock().await;
    ledger.data.brief = Some(brief.clone());
    ledger.event(
        "boardroom",
        "Boardroom brief written",
        format!(
            "{} · {} · from {payments} paid order(s)",
            brief["headline"].as_str().unwrap_or_default(),
            ai.label()
        ),
    );
    ledger.save();
    reply(StatusCode::OK, json!({ "brief": brief }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "headline": "Turn first-time buyers into repeat buyers",
            "summary": "You captured $90.00 USD from 2 orders.",
            "experiment": {
                "title": "Returning-buyer bundle", "audience": "Your 2 recent buyers",
                "offer": "A $40 bundle link", "duration": "7 days",
                "guardrail": "Stop after 5 sales", "success_measure": "Paid bundle links"
            },
            "perspectives": [
                { "role": "growth lead", "take": "Measure paid links." },
                { "role": "Strategist", "take": "Keep it small." },
                { "role": "CFO", "take": "Cap the discount." },
                { "role": "Marketer", "take": "Send a personal note." }
            ],
            "verdict": "Run it for a week.",
            "copilot_request": "Create a $40 payment link for an Everyday Tote bundle."
        })
    }

    #[test]
    fn briefs_are_validated_and_roles_put_in_a_fixed_order() {
        let text = format!("Here you go:\n```json\n{}\n```", sample());
        let brief = parse_brief(&text).unwrap();
        let roles: Vec<&str> = brief["perspectives"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ROLES);
        assert_eq!(brief["perspectives"][3]["take"], "Measure paid links.");
        assert_eq!(brief["experiment"]["successMeasure"], "Paid bundle links");
        assert_eq!(
            brief["copilotRequest"],
            "Create a $40 payment link for an Everyday Tote bundle."
        );

        let mut missing_role = sample();
        missing_role["perspectives"].as_array_mut().unwrap().pop();
        assert!(parse_brief(&missing_role.to_string()).is_err());
        let mut blank = sample();
        blank["headline"] = json!("   ");
        assert!(parse_brief(&blank.to_string()).is_err());
        assert!(parse_brief("no json here").is_err());
        assert!(parse_brief("} {").is_err());
    }

    #[test]
    fn long_or_messy_model_text_is_cleaned_and_shortened() {
        assert_eq!(fit(&json!("a\u{7}b\n  c"), 20).as_deref(), Some("a b c"));
        let long = fit(&json!("word ".repeat(40)), 30).unwrap();
        assert!(long.chars().count() <= 30);
        assert!(long.ends_with('…'));
        assert!(fit(&json!(42), 10).is_none());
    }

    #[test]
    fn prompt_offers_only_permitted_copilot_actions() {
        let basic = system_prompt(Capabilities::default());
        assert!(basic.contains("payment links"));
        assert!(!basic.contains("invoices"));
        let full = system_prompt(Capabilities {
            invoicing: true,
            transaction_search: true,
            payouts: true,
        });
        assert!(full.contains("invoices"));
    }

    #[test]
    fn facts_cover_the_last_30_days_and_all_time() {
        let data = crate::ledger::Data::default();
        let (figures, payments) = facts(&data, 1_800_000_000);
        assert_eq!(payments, 0);
        assert_eq!(figures["last_30_days"]["gross_sales"], "$0.00");
        assert_eq!(figures["all_time"]["paid_orders"], 0);
        assert_eq!(figures["business_name"], "The Boardroom Demo");
    }
}
