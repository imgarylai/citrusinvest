//! Closed-trade records produced by the NAV loop.

/// Direction of a trade — `long` when the entry weight was positive, `short`
/// when negative. Serialized lowercase (`"long"` / `"short"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeSide {
    Long,
    Short,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Trade {
    pub symbol: String,
    pub entry_date: i32,
    pub exit_date: Option<i32>,
    pub ret: f64,
    pub period: u32,
    pub mae: Option<f64>,
    pub mfe: Option<f64>,
    /// Fill price the position was opened at (the price-panel value on
    /// `entry_date`). May be `null` if that cell was missing.
    pub entry_price: f64,
    /// Fill price the position was closed at — the panel value on `exit_date`,
    /// or the last valid price less `delist_haircut` for a delisting exit.
    /// Absent for open (mark-to-market) trades.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_price: Option<f64>,
    /// `long` / `short`, from the sign of the entry weight.
    pub side: TradeSide,
}
