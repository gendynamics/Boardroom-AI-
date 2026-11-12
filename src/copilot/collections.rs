//! Collections run: one reviewed batch of payment reminders for overdue invoices.
//!
//! The server, not the model, decides which invoices are in the run and in what order: it reads
//! the invoices from PayPal, keeps the overdue ones this app created, and ranks them by balance.
//! A second, narrow AI call only writes the wording (reason, subject, note) for that fixed list.
//! Its output is rejected unless it covers exactly the same invoices in the same order and
//! states no number that is absent from the input. Nothing is sent until the merchant approves,
//! and each invoice is re-read from PayPal at that moment so a paid invoice is never chased.

use std::collections::BTreeSet;

use super::*;
use crate::valid_invoice_id;

/// Most invoices in one run.
pub(crate) const MAX_RUN: usize = 10;
const WRITER_ATTEMPTS: usize = 2;
const MAX_SUBJECT_CHARS: usize = 69;
const MAX_REASON_CHARS: usize = 120;
const MAX_NOTE_CHARS: usize = 600;

const WRITER_PROMPT: &str = "You write payment reminder notes for a small business.
You will receive a JSON list of overdue invoices. For each one, return:
- invoice_id (copied exactly from the input)
- reason: one short phrase on why it ranks where it does
- subject: under 70 characters
- note: 2-3 sentences, polite, specific to the days overdue.
  Under 14 days: friendly nudge. 14-30: firm and clear. Over 30: direct,
  asks for a reply with a payment date.
Never state an amount, date, or name that is not in the input.
Never add, remove, or reorder invoices.
Return JSON only: {\"items\":[{invoice_id, reason, subject, note}]}";

/// One reminder in a run. PayPal supplies the facts; the AI supplies reason, subject, and note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollectionItem {
    pub(crate) invoice_id: String,
    pub(crate) recipient: String,
    pub(crate) due_cents: i64,
    pub(crate) days_overdue: i64,
    pub(crate) reason: String,
    pub(crate) subject: String,
    pub(crate) note: String,
    pub(crate) selected: bool,
}

/// An overdue invoice as PayPal reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Overdue {
    pub(crate) invoice_id: String,
    number: Option<String>,
    recipient: String,
    recipient_name: Option<String>,
    pub(crate) due_cents: i64,
    due_date: String,
    pub(crate) days_overdue: i64,
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested)

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "YYYY-MM-DD" -> days since the Unix epoch.
fn date_days(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let part = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (part(0..4)?, part(5..7)?, part(8..10)?);
    ((1..=12).contains(&month) && (1..=31).contains(&day))
        .then(|| days_from_civil(year, month, day))
}

/// Checks that an invoice can be chased today: still open, past due, USD, with a balance, and
/// addressed to a Sandbox test recipient.
pub(crate) fn overdue_invoice(invoice: &Value, today: &str) -> Result<Overdue, String> {
    let invoice_id = invoice["id"]
        .as_str()
        .filter(|id| valid_invoice_id(id))
        .ok_or("PayPal returned an invoice without a usable ID.")?;
    let status = invoice["status"].as_str().unwrap_or("UNKNOWN");
    if !OUTSTANDING_STATUSES.contains(&status) {
        return Err(format!("{invoice_id} is {status}, so it no longer needs a reminder."));
    }
    let due_date = invoice["detail"]["payment_term"]["due_date"]
        .as_str()
        .unwrap_or_default();
    let days_overdue = match (date_days(today), date_days(due_date)) {
        (Some(now), Some(due)) => now - due,
        _ => return Err(format!("{invoice_id} has no due date.")),
    };
    if days_overdue < 1 {
        return Err(format!("{invoice_id} is not overdue."));
    }
    let (due_cents, currency) = money_value(&invoice["due_amount"])
        .or_else(|| money_value(&invoice["amount"]))
        .ok_or_else(|| format!("{invoice_id} has no amount due."))?;
    if currency != "USD" || due_cents <= 0 {
        return Err(format!("{invoice_id} has no USD balance due."));
    }
    let billing = &invoice["primary_recipients"][0]["billing_info"];
    let recipient = billing["email_address"]
        .as_str()
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| valid_test_recipient(email))
        .ok_or_else(|| format!("{invoice_id} is not addressed to a Sandbox test recipient."))?;
    let text = |value: &Value, max: usize| {
        excerpt(value, max)
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Ok(Overdue {
        invoice_id: invoice_id.to_owned(),
        number: text(&invoice["detail"]["invoice_number"], 40),
        recipient,
        recipient_name: text(&billing["name"]["given_name"], 40),
        due_cents,
        due_date: due_date.to_owned(),
        days_overdue,
    })
}

/// Largest balance first, then longest overdue.
pub(crate) fn rank(invoices: &mut [Overdue]) {
    invoices.sort_by(|a, b| {
        b.due_cents
            .cmp(&a.due_cents)
            .then(b.days_overdue.cmp(&a.days_overdue))
            .then(a.invoice_id.cmp(&b.invoice_id))
    });
}

fn writer_row(invoice: &Overdue) -> Value {
    let mut row = json!({
        "invoice_id": invoice.invoice_id,
        "recipient_email": invoice.recipient,
        "amount_due": decimal(invoice.due_cents),
        "currency": "USD",
        "due_date": invoice.due_date,
        "days_overdue": invoice.days_overdue
    });
    if let Some(number) = &invoice.number {
        row["invoice_number"] = json!(number);
    }
    if let Some(name) = &invoice.recipient_name {
        row["recipient_name"] = json!(name);
    }
    row
}

/// Runs of digits (with inner "." or ",") in a text, with thousands separators removed.
fn number_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || (matches!(ch, '.' | ',') && !current.is_empty()) {
            current.push(ch);
        } else if !current.is_empty() {
            let token = current.trim_end_matches(['.', ',']).replace(',', "");
            tokens.push(token);
            current.clear();
        }
    }
    tokens
}

/// Every number the writer may repeat for one invoice.
fn allowed_numbers(row: &Value) -> BTreeSet<String> {
    let mut allowed = BTreeSet::new();
    for value in row.as_object().into_iter().flat_map(|map| map.values()) {
        let text = match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        for token in number_tokens(&text) {
            if let Some(whole) = token.strip_suffix(".00") {
                allowed.insert(whole.to_owned());
            }
            allowed.insert(token.trim_start_matches('0').to_owned());
            allowed.insert(token);
        }
    }
    allowed
}

fn ungrounded_number(text: &str, allowed: &BTreeSet<String>) -> Option<String> {
    number_tokens(text).into_iter().find(|token| {
        !allowed.contains(token) && !allowed.contains(token.trim_start_matches('0'))
    })
}

/// Turns the writer's reply into reminders, or says why it cannot be used.
pub(crate) fn parse_notes(text: &str, invoices: &[Overdue]) -> Result<Vec<CollectionItem>, String> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err("The AI did not return JSON.".to_owned());
    };
    let raw: Value = serde_json::from_str(&text[start..=end])
        .map_err(|_| "The AI did not return valid JSON.".to_owned())?;
    let rows = raw["items"]
        .as_array()
        .filter(|rows| rows.len() == invoices.len())
        .ok_or("The AI added or removed invoices.")?;
    let mut items = Vec::with_capacity(invoices.len());
    for (row, invoice) in rows.iter().zip(invoices) {
        let id = &invoice.invoice_id;
        if row["invoice_id"].as_str() != Some(id.as_str()) {
            return Err("The AI reordered or changed the invoices.".to_owned());
        }
        let reason = clean_text(&row["reason"], MAX_REASON_CHARS, false)
            .ok_or_else(|| format!("The AI gave no usable reason for {id}."))?;
        let subject = clean_text(&row["subject"], MAX_SUBJECT_CHARS, false)
            .ok_or_else(|| format!("The AI's subject for {id} was missing or 70+ characters."))?;
        let note = clean_text(&row["note"], MAX_NOTE_CHARS, true)
            .ok_or_else(|| format!("The AI's note for {id} was missing or too long."))?;
        let allowed = allowed_numbers(&writer_row(invoice));
        for written in [&reason, &subject, &note] {
            if let Some(number) = ungrounded_number(written, &allowed) {
                return Err(format!(
                    "The AI's wording for {id} states a number ({number}) that is not in PayPal's data."
                ));
            }
        }
        items.push(CollectionItem {
            invoice_id: id.clone(),
            recipient: invoice.recipient.clone(),
            due_cents: invoice.due_cents,
            days_overdue: invoice.days_overdue,
            reason,
            subject,
            note,
            selected: true,
        });
    }
    Ok(items)
}

pub(crate) fn selected_total(items: &[CollectionItem]) -> (usize, i64) {
    let selected = items.iter().filter(|item| item.selected);
    (selected.clone().count(), selected.map(|item| item.due_cents).sum())
}

/// The run as the review card and the model see it.
pub(crate) fn payload(proposal_id: &str, items: &[CollectionItem]) -> Value {
    let (count, cents) = selected_total(items);
    json!({
        "type": "collections_run",
        "proposal_id": proposal_id,
        "totals": { "count": count, "amount_due": decimal(cents), "currency": "USD" },
        "items": items.iter().map(|item| json!({
            "invoice_id": item.invoice_id,
            "recipient": item.recipient,
            "amount_due": decimal(item.due_cents),
            "days_overdue": item.days_overdue,
            "reason": item.reason,
            "subject": item.subject,
            "note": item.note,
            "selected": item.selected
        })).collect::<Vec<_>>()
    })
}

// ---------------------------------------------------------------------------------------------
// PayPal and AI calls

/// Whether this app created the invoice and it has been sent.
async fn managed(state: &AppState, invoice_id: &str) -> bool {
    state
        .ledger
        .lock()
        .await
        .invoice_mut(invoice_id)
        .is_some_and(|record| !matches!(record.status.as_str(), "draft" | "cancelled"))
}

async fn write_notes(state: &AppState, invoices: &[Overdue]) -> Result<Vec<CollectionItem>, String> {
    let Some(ai) = state.ai.clone() else {
        return Err("The AI is not configured, so reminder notes cannot be written.".to_owned());
    };
    let input = Value::Array(invoices.iter().map(writer_row).collect());
    let messages = [
        json!({ "role": "system", "content": WRITER_PROMPT }),
        json!({ "role": "user", "content": input.to_string() }),
    ];
    let mut last_error = String::new();
    for _ in 0..WRITER_ATTEMPTS {
        let message = llm::chat(&state.client, &ai, &messages, &[])
            .await
            .map_err(str::to_owned)?;
        match parse_notes(&message_text(&message), invoices) {
            Ok(items) => return Ok(items),
            Err(error) => {
                eprintln!("Collections notes rejected: {error}");
                last_error = error;
            }
        }
    }
    Err(format!("{last_error} No reminders were drafted; try again."))
}

pub(super) async fn propose_run(state: &AppState) -> ToolOutcome {
    const LABEL: &str = "Collections run not drafted";
    let payload = match paypal_api_request(
        state,
        Method::GET,
        "/v2/invoicing/invoices?page=1&page_size=100&total_required=true",
        Value::Null,
    )
    .await
    {
        Ok(payload) => payload,
        Err(error) => return ToolOutcome::paypal(error, LABEL),
    };
    let today = today();
    let mut invoices = Vec::new();
    let mut not_from_this_app = 0;
    for invoice in payload["items"].as_array().into_iter().flatten() {
        let Ok(overdue) = overdue_invoice(invoice, &today) else {
            continue;
        };
        if managed(state, &overdue.invoice_id).await {
            invoices.push(overdue);
        } else {
            not_from_this_app += 1;
        }
    }
    if invoices.is_empty() {
        return ToolOutcome::ok(
            json!({
                "overdue_invoices_to_remind": 0,
                "overdue_invoices_not_created_by_this_app": not_from_this_app,
                "note": "Nothing to chase. Only overdue USD invoices created and sent through this app can be reminded here."
            }),
            "No overdue invoices to chase",
        );
    }
    rank(&mut invoices);
    let left_out = invoices.len().saturating_sub(MAX_RUN);
    invoices.truncate(MAX_RUN);
    let items = match write_notes(state, &invoices).await {
        Ok(items) => items,
        Err(message) => return ToolOutcome::error(message, LABEL),
    };
    let mut outcome = propose(state, ActionKind::CollectionsRun { items }).await;
    if left_out > 0 {
        outcome.result["overdue_invoices_left_out"] = json!(left_out);
    }
    outcome
}

/// Sends the selected reminders. Each invoice is re-read first; one that was paid, cancelled,
/// or is otherwise no longer overdue is skipped rather than chased.
pub(super) async fn execute_run(
    state: &AppState,
    action_id: &str,
    items: &[CollectionItem],
) -> Result<Value, String> {
    let today = today();
    let mut results = Vec::new();
    let mut reminded = 0;
    let mut first_problem = None;
    for (index, item) in items.iter().enumerate().filter(|(_, item)| item.selected) {
        let id = &item.invoice_id;
        let check = if !managed(state, id).await {
            Err(format!("{id} can no longer be changed from this app."))
        } else {
            match paypal_api_request(
                state,
                Method::GET,
                &format!("/v2/invoicing/invoices/{id}"),
                Value::Null,
            )
            .await
            {
                Ok(invoice) => overdue_invoice(&invoice, &today).map(|_| ()),
                Err(error) => Err(error.message.to_owned()),
            }
        };
        let (outcome, message) = match check {
            Err(message) => ("skipped", Some(message)),
            Ok(()) => match paypal_api_request_with_id(
                state,
                Method::POST,
                &format!("/v2/invoicing/invoices/{id}/remind"),
                json!({
                    "subject": item.subject,
                    "note": item.note,
                    "send_to_invoicer": false,
                    "send_to_recipient": true
                }),
                &format!("{action_id}-{index}"),
            )
            .await
            {
                Ok(_) => ("reminded", None),
                Err(error) => ("failed", Some(error.message.to_owned())),
            },
        };
        if outcome == "reminded" {
            reminded += 1;
            let mut ledger = state.ledger.lock().await;
            ledger.event("invoice", "Invoice reminder sent", id.clone());
            ledger.save();
        } else if first_problem.is_none() {
            first_problem = message.clone();
        }
        results.push(json!({ "invoice_id": id, "outcome": outcome, "message": message }));
    }
    if reminded == 0 {
        return Err(format!(
            "No reminders were sent. {}",
            first_problem.unwrap_or_else(|| "No invoices were selected.".to_owned())
        ));
    }
    Ok(json!({ "reminded": reminded, "requested": results.len(), "results": results }))
}
