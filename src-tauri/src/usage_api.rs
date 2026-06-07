//! 用量统计对前端的视图层：把持久化的聚合数据整理成有序列表。

use crate::store::{self, UsageTotals};
use chrono::Local;
use serde::Serialize;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize)]
pub struct ModelUsageView {
    pub model: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyUsageView {
    pub date: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

/// 用量总览：累计用量 + 按模型拆分 + 活跃日历。
#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    pub total: UsageTotals,
    pub models: Vec<ModelUsageView>,
    /// 按日期升序排列的每日用量，构成「活跃日历」数据源。
    pub calendar: Vec<DailyUsageView>,
    /// 有记录（requests > 0）的天数。
    pub active_days: u64,
    pub first_recorded_at: Option<String>,
    pub last_recorded_at: Option<String>,
    /// 本地当天日期（YYYY-MM-DD），便于前端高亮今天。
    pub today: String,
}

pub fn get_usage_summary(app: &AppHandle) -> Result<UsageSummary, String> {
    let store = store::load_usage(app)?;

    let mut models: Vec<ModelUsageView> = store
        .models
        .into_iter()
        .map(|(model, totals)| ModelUsageView { model, totals })
        .collect();
    models.sort_by(|a, b| {
        b.totals
            .cost_usd
            .partial_cmp(&a.totals.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.totals.total_tokens().cmp(&a.totals.total_tokens()))
    });

    let mut calendar: Vec<DailyUsageView> = store
        .days
        .into_iter()
        .map(|(date, day)| DailyUsageView {
            date,
            totals: day.totals,
        })
        .collect();
    calendar.sort_by(|a, b| a.date.cmp(&b.date));

    let active_days = calendar
        .iter()
        .filter(|day| day.totals.requests > 0)
        .count() as u64;

    Ok(UsageSummary {
        total: store.total,
        models,
        calendar,
        active_days,
        first_recorded_at: store.first_recorded_at,
        last_recorded_at: store.last_recorded_at,
        today: Local::now().format("%Y-%m-%d").to_string(),
    })
}

pub fn clear_usage(app: &AppHandle) -> Result<(), String> {
    store::clear_usage(app)
}
