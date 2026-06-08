use std::time::Duration;

pub async fn fetch_latest_version() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let resp = client
        .get("https://registry.npmjs.org/share-anything-cli/latest")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("version")?.as_str().map(|s| s.to_string())
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.')
            .map(|p| p.split('-').next().unwrap_or(p))
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let l = parse(latest);
    let c = parse(current);
    let len = l.len().max(c.len());
    for i in 0..len {
        let a = l.get(i).copied().unwrap_or(0);
        let b = c.get(i).copied().unwrap_or(0);
        if a > b {
            return true;
        }
        if a < b {
            return false;
        }
    }
    false
}

pub async fn check_for_update() -> Option<String> {
    let latest = fetch_latest_version().await?;
    let current = env!("CARGO_PKG_VERSION");
    if is_newer(&latest, current) {
        Some(latest)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn detects_newer_patch() {
        assert!(is_newer("0.5.8", "0.5.7"));
        assert!(is_newer("0.5.10", "0.5.7"));
    }

    #[test]
    fn detects_newer_minor() {
        assert!(is_newer("0.6.0", "0.5.99"));
    }

    #[test]
    fn equal_is_not_newer() {
        assert!(!is_newer("0.5.7", "0.5.7"));
    }

    #[test]
    fn older_is_not_newer() {
        assert!(!is_newer("0.5.6", "0.5.7"));
    }
}
