use chrono::{Local, NaiveDateTime, TimeZone, Utc};

pub fn utc_to_local(utc_str: &str) -> String {
    NaiveDateTime::parse_from_str(utc_str, "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|naive| Utc.from_local_datetime(&naive).single())
        .map(|utc_dt| utc_dt.with_timezone(&Local).format("%Y-%m-%d %H:%M %Z").to_string())
        .unwrap_or_else(|| utc_str.to_string())
}
