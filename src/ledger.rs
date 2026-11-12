//! Persistent record of what this app has done with PayPal Sandbox.
//!
//! PayPal Orders v2 has no "list my orders" endpoint, and Transaction Search needs an extra
//! permission and can lag by hours. So the app keeps its own record of the orders, captures,
//! refunds, invoices, and payouts it creates. The dashboard, activity, wallet, and audit pages
//! read it. Every amount comes from a PayPal response, or from the exact request PayPal
//! accepted. It is one JSON file, rewritten atomically (temporary file + rename) after changes.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::copilot::now_seconds;

const MAX_ORDERS: usize = 500;
const MAX_PAYOUTS: usize = 200;
const MAX_INVOICES: usize = 200;
const MAX_EVENTS: usize = 300;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Profile {
    pub display_name: String,
    /// Shown to buyers on PayPal checkout pages (`brand_name`).
    pub business_name: String,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            display_name: "Morgan".to_owned(),
            business_name: "The Boardroom Demo".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefundRecord {
    pub id: Option<String>,
    pub at: i64,
    pub cents: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrderRecord {
    pub id: String,
    pub created_at: i64,
    /// checkout, card, payment_link (Copilot), or request (Send & request).
    pub source: String,
    pub description: String,
    pub amount_cents: i64,
    pub currency: String,
    /// created, capture_pending, captured, or capture_unknown.
    pub state: String,
    /// paypal, card_when_required, or card_always.
    pub verification: String,
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(default)]
    pub approval_url: Option<String>,
    #[serde(default)]
    pub captured_at: Option<i64>,
    #[serde(default)]
    pub capture_id: Option<String>,
    #[serde(default)]
    pub captured_cents: i64,
    #[serde(default)]
    pub payer_email: Option<String>,
    #[serde(default)]
    pub payer_name: Option<String>,
    #[serde(default)]
    pub refunds: Vec<RefundRecord>,
}

impl OrderRecord {
    pub fn refunded_cents(&self) -> i64 {
        self.refunds.iter().map(|refund| refund.cents).sum()
    }

    pub fn is_paid(&self) -> bool {
        self.state == "captured" && self.captured_cents > 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayoutRecord {
    pub batch_id: String,
    pub request_id: String,
    pub created_at: i64,
    pub recipient: String,
    pub amount_cents: i64,
    #[serde(default)]
    pub note: Option<String>,
    /// PayPal's batch status (PENDING, PROCESSING, SUCCESS, ...) or item status once known.
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvoiceRecord {
    pub id: String,
    pub created_at: i64,
    pub recipient: String,
    pub amount_cents: i64,
    /// draft, send_pending, sent, send_unknown, cancelled, or a PayPal status such as PAID.
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub at: i64,
    pub kind: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub profile: Profile,
    pub orders: Vec<OrderRecord>,
    pub payouts: Vec<PayoutRecord>,
    pub invoices: Vec<InvoiceRecord>,
    pub events: Vec<Event>,
    /// The last Boardroom brief the AI produced, so it survives a reload.
    pub brief: Option<Value>,
}

/// The record plus where it lives. Tests use the default, which never touches the disk.
#[derive(Default)]
pub struct Store {
    path: Option<PathBuf>,
    pub data: Data,
}

impl Store {
    /// Loads the record, or starts an empty one if the file does not exist yet. A file that
    /// exists but cannot be parsed is an error: starting fresh would silently overwrite it.
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let data = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                format!(
                    "Could not parse the activity record {}: {error}. Move the file aside to start a fresh record.",
                    path.display()
                )
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => Data::default(),
            Err(error) => {
                return Err(format!(
                    "Could not read the activity record {}: {error}",
                    path.display()
                ));
            }
        };
        Ok(Self {
            path: Some(path),
            data,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Trims old entries and writes the record. A failed write is logged, not fatal: PayPal
    /// already holds the authoritative copy of anything recorded here.
    pub fn save(&mut self) {
        trim_front(&mut self.data.orders, MAX_ORDERS);
        trim_front(&mut self.data.payouts, MAX_PAYOUTS);
        trim_front(&mut self.data.invoices, MAX_INVOICES);
        trim_front(&mut self.data.events, MAX_EVENTS);
        let Some(path) = &self.path else {
            return;
        };
        if let Err(error) = write_atomically(path, &self.data) {
            eprintln!(
                "Could not save the activity record to {}: {error}",
                path.display()
            );
        }
    }

    pub fn order_mut(&mut self, id: &str) -> Option<&mut OrderRecord> {
        self.data.orders.iter_mut().rev().find(|order| order.id == id)
    }

    pub fn invoice_mut(&mut self, id: &str) -> Option<&mut InvoiceRecord> {
        self.data
            .invoices
            .iter_mut()
            .rev()
            .find(|invoice| invoice.id == id)
    }

    pub fn payout_mut(&mut self, batch_id: &str) -> Option<&mut PayoutRecord> {
        self.data
            .payouts
            .iter_mut()
            .rev()
            .find(|payout| payout.batch_id == batch_id)
    }

    /// Adds an audit-trail entry. Callers save afterwards.
    pub fn event(&mut self, kind: &str, title: impl Into<String>, detail: impl Into<String>) {
        self.data.events.push(Event {
            at: now_seconds(),
            kind: kind.to_owned(),
            title: title.into(),
            detail: detail.into(),
        });
    }
}

fn trim_front<T>(items: &mut Vec<T>, max: usize) {
    if items.len() > max {
        let excess = items.len() - max;
        items.drain(..excess);
    }
}

fn write_atomically(path: &Path, data: &Data) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(data).map_err(std::io::Error::other)?;
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(id: &str) -> OrderRecord {
        OrderRecord {
            id: id.to_owned(),
            created_at: 1,
            source: "checkout".to_owned(),
            description: "Test".to_owned(),
            amount_cents: 1_000,
            currency: "USD".to_owned(),
            state: "created".to_owned(),
            verification: "paypal".to_owned(),
            customer: None,
            approval_url: None,
            captured_at: None,
            capture_id: None,
            captured_cents: 0,
            payer_email: None,
            payer_name: None,
            refunds: Vec::new(),
        }
    }

    #[test]
    fn record_survives_a_restart_and_never_overwrites_a_corrupt_file() {
        let directory = std::env::temp_dir().join(format!("boardroom-ledger-{}", uuid::Uuid::new_v4()));
        let path = directory.join("ledger.json");

        let mut store = Store::open(path.clone()).unwrap();
        assert!(store.data.orders.is_empty());
        store.data.orders.push(order("ORDER0000000001"));
        store.data.profile.business_name = "Avery's Bags".to_owned();
        store.event("order", "Created", "detail");
        store.save();
        assert!(!path.with_extension("json.tmp").exists());

        let reopened = Store::open(path.clone()).unwrap();
        assert_eq!(reopened.data.orders, vec![order("ORDER0000000001")]);
        assert_eq!(reopened.data.profile.business_name, "Avery's Bags");
        assert_eq!(reopened.data.events.len(), 1);

        fs::write(&path, "{ not json").unwrap();
        let Err(message) = Store::open(path.clone()) else {
            panic!("a corrupt record must not open");
        };
        assert!(message.contains("Move the file aside"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn old_entries_are_trimmed_oldest_first() {
        let mut store = Store::default();
        for index in 0..(MAX_EVENTS + 5) {
            store.event("test", format!("event {index}"), "");
        }
        store.save();
        assert_eq!(store.data.events.len(), MAX_EVENTS);
        assert_eq!(store.data.events[0].title, "event 5");
    }

    #[test]
    fn refunds_add_up_and_paid_means_captured() {
        let mut record = order("ORDER0000000002");
        assert!(!record.is_paid());
        record.state = "captured".to_owned();
        record.captured_cents = 4_500;
        record.refunds.push(RefundRecord { id: None, at: 2, cents: 500 });
        record.refunds.push(RefundRecord { id: None, at: 3, cents: 250 });
        assert!(record.is_paid());
        assert_eq!(record.refunded_cents(), 750);
    }
}
