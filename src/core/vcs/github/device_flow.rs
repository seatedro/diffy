use serde::Serialize;

use crate::core::error::{DiffyError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceFlowState {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u32,
}

pub fn start_device_flow(client_id: &str) -> Result<DeviceFlowState> {
    let body = ureq::post("https://github.com/login/device/code")
        .header("Accept", "application/x-www-form-urlencoded")
        .header("User-Agent", "diffy/0.1")
        .send_form([("client_id", client_id), ("scope", "repo")])?
        .into_body()
        .read_to_string()
        .map_err(|error| DiffyError::Http(error.to_string()))?;

    let state = DeviceFlowState {
        device_code: form_value(&body, "device_code")
            .unwrap_or_default()
            .to_owned(),
        user_code: form_value(&body, "user_code")
            .unwrap_or_default()
            .to_owned(),
        verification_uri: decode_form_value(
            form_value(&body, "verification_uri").unwrap_or_default(),
        ),
        interval: form_value(&body, "interval")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(5)
            .max(5),
    };

    if state.device_code.is_empty() || state.user_code.is_empty() {
        return Err(DiffyError::Parse(
            "invalid GitHub device flow response".to_owned(),
        ));
    }

    Ok(state)
}

pub fn poll_for_token(client_id: &str, device_code: &str) -> Result<Option<String>> {
    let body = ureq::post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/x-www-form-urlencoded")
        .header("User-Agent", "diffy/0.1")
        .send_form([
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])?
        .into_body()
        .read_to_string()
        .map_err(|error| DiffyError::Http(error.to_string()))?;

    match form_value(&body, "error") {
        Some("authorization_pending") | Some("slow_down") => Ok(None),
        Some("expired_token") => Err(DiffyError::Http("device code expired".to_owned())),
        Some(other) => {
            let description = form_value(&body, "error_description")
                .map(decode_form_value)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| other.to_owned());
            Err(DiffyError::Http(description))
        }
        None => {
            let token = form_value(&body, "access_token").unwrap_or_default();
            if token.is_empty() {
                Err(DiffyError::Parse(
                    "missing access token in device flow response".to_owned(),
                ))
            } else {
                Ok(Some(token.to_owned()))
            }
        }
    }
}

fn form_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == key).then_some(value)
    })
}

fn decode_form_value(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                result.push(' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    result.push(byte as char);
                    index += 3;
                } else {
                    result.push('%');
                    index += 1;
                }
            }
            byte => {
                result.push(byte as char);
                index += 1;
            }
        }
    }
    result
}
