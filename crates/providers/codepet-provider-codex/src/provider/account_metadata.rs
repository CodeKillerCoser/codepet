use super::*;

pub(super) fn codex_authentication(response: &Value) -> Option<ProviderAuthentication> {
    let requires_auth = response
        .get("requiresOpenaiAuth")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let Some(account) = response.get("account").filter(|value| !value.is_null()) else {
        return Some(ProviderAuthentication {
            status: if requires_auth {
                ProviderAuthenticationStatus::SignedOut
            } else {
                ProviderAuthenticationStatus::Unsupported
            },
            display_text: Some(if requires_auth {
                "Not signed in".to_string()
            } else {
                "Authentication not required".to_string()
            }),
        });
    };
    let account_type = account
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let display_text = match account_type {
        "chatgpt" => account
            .get("planType")
            .and_then(Value::as_str)
            .map(|plan| format!("Signed in · ChatGPT {}", display_plan_name(plan)))
            .unwrap_or_else(|| "Signed in · ChatGPT".to_string()),
        "apiKey" => "Signed in · API key".to_string(),
        "amazonBedrock" => "Signed in · Amazon Bedrock".to_string(),
        _ => "Signed in".to_string(),
    };
    Some(ProviderAuthentication {
        status: ProviderAuthenticationStatus::SignedIn,
        display_text: Some(display_text),
    })
}

fn display_plan_name(plan: &str) -> String {
    plan.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn codex_usage(
    rate_limits: Option<&Value>,
    token_usage: Option<&Value>,
) -> Option<ProviderUsage> {
    let mut details = Vec::new();
    let mut display_parts = Vec::new();
    if let Some(rate_limits) = rate_limits {
        let mut data = codepet_provider_sdk::JsonObject::new();
        for key in ["rateLimits", "rateLimitsByLimitId", "rateLimitResetCredits"] {
            if let Some(value) = rate_limits.get(key).filter(|value| !value.is_null()) {
                data.insert(key.to_string(), value.clone());
            }
        }
        if !data.is_empty() {
            details.push(ProviderUsageDetail {
                namespace: "openai.codex.rate-limits".to_string(),
                schema_version: "1".to_string(),
                data,
            });
        }
        if let Some(snapshot) = preferred_rate_limit_snapshot(rate_limits) {
            for key in ["primary", "secondary"] {
                if let Some(window) = snapshot.get(key).filter(|value| !value.is_null()) {
                    if let Some(label) = rate_limit_window_label(window) {
                        display_parts.push(label);
                    }
                }
            }
        }
    }
    if let Some(summary) = token_usage
        .and_then(|usage| usage.get("summary"))
        .filter(|value| value.is_object())
    {
        details.push(ProviderUsageDetail {
            namespace: "openai.codex.token-usage".to_string(),
            schema_version: "1".to_string(),
            data: [("summary".to_string(), summary.clone())]
                .into_iter()
                .collect(),
        });
        if display_parts.is_empty() {
            if let Some(tokens) = summary.get("lifetimeTokens").and_then(Value::as_u64) {
                display_parts.push(format!("{} lifetime tokens", compact_number(tokens)));
            }
        }
    }
    if details.is_empty() {
        return None;
    }
    Some(ProviderUsage {
        display_text: if display_parts.is_empty() {
            "Usage data available".to_string()
        } else {
            display_parts.join(" · ")
        },
        observed_at: Some(now_ms()),
        details: Some(details),
    })
}

fn preferred_rate_limit_snapshot(response: &Value) -> Option<&Value> {
    response
        .pointer("/rateLimitsByLimitId/codex")
        .or_else(|| response.get("rateLimits"))
}

fn rate_limit_window_label(window: &Value) -> Option<String> {
    let used = window.get("usedPercent")?.as_u64()?.min(100);
    let remaining = 100_u64.saturating_sub(used);
    let duration = window
        .get("windowDurationMins")
        .and_then(Value::as_u64)
        .map(format_window_duration)
        .unwrap_or_else(|| "Quota".to_string());
    Some(format!("{duration} {remaining}% remaining"))
}

fn format_window_duration(minutes: u64) -> String {
    if minutes > 0 && minutes % (24 * 60) == 0 {
        format!("{}d", minutes / (24 * 60))
    } else if minutes > 0 && minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

fn compact_number(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}
