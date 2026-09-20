use serde_json::Value;

use crate::error::{hints, HsrError, HsrResult};

/// Shared structural privacy gate for every committed JSON input boundary.
/// Values are never included in an error; only the prohibited field path is.
pub(crate) fn reject_sensitive_fields(value: &Value) -> HsrResult<()> {
    reject_at(value, "$")
}

fn reject_at(value: &Value, path: &str) -> HsrResult<()> {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let normalized = normalize_key(key);
                if is_sensitive_key(&normalized) {
                    return Err(HsrError::new(
                        "HSR-DATA-SENSITIVE",
                        hints::SENSITIVE_DATA,
                        format!("prohibited field at {path}.{key}"),
                    ));
                }
                reject_at(child, &format!("{path}.{key}"))?;
            }
        },
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                reject_at(child, &format!("{path}[{index}]"))?;
            }
        },
        _ => {},
    }
    Ok(())
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_sensitive_key(normalized: &str) -> bool {
    const EXACT: &[&str] = &[
        "uid",
        "localid",
        "locationkey",
        "userid",
        "useruid",
        "playeruid",
        "guid",
        "uniqueid",
        "token",
        "accesstoken",
        "refreshtoken",
        "cookie",
        "cookies",
        "session",
        "sessionid",
        "account",
        "accountid",
        "accountuid",
        "playername",
        "nickname",
    ];
    EXACT.contains(&normalized)
        || normalized.ends_with("uid")
        || normalized.ends_with("guid")
        || normalized.ends_with("token")
        || normalized.ends_with("cookie")
        || normalized.ends_with("sessionid")
}
