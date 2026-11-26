use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

const MAX_CAPTURE: usize = 544 * 1024;
const MAX_RECORDS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capture {
    pub name: String,
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    schema: u8,
    app: String,
    started_unix_ms: u64,
    #[serde(rename = "vendor_id")]
    _vendor_id: u16,
    #[serde(rename = "product_id")]
    _product_id: u16,
    #[serde(rename = "usage_page")]
    _usage_page: u16,
    #[serde(rename = "usage")]
    _usage: u16,
    #[serde(rename = "input_bytes")]
    _input_bytes: u16,
    #[serde(rename = "output_bytes")]
    _output_bytes: u16,
    #[serde(rename = "feature_bytes")]
    _feature_bytes: u16,
    scope: String,
    privacy: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Record {
    Transfer {
        elapsed_ms: u64,
        api: String,
        #[serde(rename = "success")]
        _success: bool,
        hex: String,
    },
    Step { elapsed_ms: u64, step: usize, label: String },
    End { reason: String, transfers: usize },
}

fn invalid() -> String {
    super::message("原始采集文件无效或超过 544 KiB，请重新采集后重试。",
        "The raw capture is invalid or exceeds 544 KiB. Capture again and retry.")
}

fn metadata_text(value: &str, maximum: usize) -> bool {
    value.encode_utf16().count() <= maximum * 2 && super::text(value.trim(), 1, maximum, false)
}

fn valid_bytes(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > MAX_CAPTURE { return false; }
    let Ok(raw) = std::str::from_utf8(bytes) else { return false; };
    let mut lines = raw.lines();
    let Some(first) = lines.next() else { return false; };
    let Ok(header) = serde_json::from_str::<Header>(first) else { return false; };
    if header.schema != 1 || !metadata_text(&header.app, 80)
        || header.started_unix_ms > super::MAX_NUMBER
        || !metadata_text(&header.scope, 512) || !metadata_text(&header.privacy, 1024) {
        return false;
    }
    let mut records = 0;
    let mut transfers = 0;
    let mut has_data = false;
    while let Some(line) = lines.next() {
        let Ok(record) = serde_json::from_str::<Record>(line) else { return false; };
        let elapsed = match record {
            Record::Transfer { elapsed_ms, api, hex, .. } => {
                if !matches!(api.as_str(), "input" | "input-timeout" | "get-input" | "get-feature"
                    | "set-feature" | "set-output" | "write")
                    || hex.len() > 8192 || hex.len() % 2 != 0
                    || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) { return false; }
                transfers += 1;
                has_data |= !hex.is_empty();
                elapsed_ms
            }
            Record::Step { elapsed_ms, step, label } => {
                if step > 4 || !metadata_text(&label, 120) { return false; }
                elapsed_ms
            }
            Record::End { reason, transfers: expected } => {
                return has_data && expected == transfers
                    && metadata_text(&reason, 80)
                    && lines.next().is_none();
            }
        };
        records += 1;
        if records > MAX_RECORDS || elapsed > super::MAX_NUMBER { return false; }
    }
    false
}

pub(super) fn validate(attachment: &Capture) -> Result<(), String> {
    if attachment.name != "raw.jsonl" || attachment.mime_type != "application/x-ndjson"
        || attachment.data_base64.len() > MAX_CAPTURE.div_ceil(3) * 4 { return Err(invalid()); }
    let bytes = STANDARD.decode(&attachment.data_base64).map_err(|_| invalid())?;
    if STANDARD.encode(&bytes) != attachment.data_base64 || !valid_bytes(&bytes) { return Err(invalid()); }
    Ok(())
}

pub fn capture(bytes: &[u8]) -> Result<Capture, String> {
    if !valid_bytes(bytes) { return Err(invalid()); }
    Ok(Capture { name: "raw.jsonl".into(), mime_type: "application/x-ndjson".into(),
        data_base64: STANDARD.encode(bytes) })
}

#[cfg(test)]
mod tests {
    use super::*;
    const SCOPE: &str = "one interface; all receiver slots; existing application I/O only; timestamps are completion times";
    const STEPS: [&str; 5] = ["Normal use", "Connect charging cable", "Disconnect charging cable",
        "Power off peripheral", "Power on peripheral"];
    const PRIVACY: &str = "Raw data may contain device identifiers or input information. Included captures are shared publicly with feedback.";
    const LEGACY_PRIVACY: &str = "LOCAL RAW DATA: may contain device identifiers or input data. Review before sharing. Not attached to feedback.";

    fn header() -> serde_json::Value {
        serde_json::json!({"schema":1, "app":"1.4.10", "started_unix_ms":1795651200000u64,
            "vendor_id":4660, "product_id":22136, "usage_page":65280, "usage":1,
            "input_bytes":64, "output_bytes":64, "feature_bytes":0, "scope":SCOPE, "privacy":PRIVACY})
    }

    fn transfer() -> serde_json::Value {
        serde_json::json!({"kind":"transfer", "elapsed_ms":100, "api":"input", "success":true, "hex":"00ABFF"})
    }

    fn end(transfers: usize) -> serde_json::Value {
        serde_json::json!({"kind":"end", "reason":"stopped", "transfers":transfers})
    }

    fn jsonl(values: &[serde_json::Value]) -> Vec<u8> {
        values.iter().map(|value| format!("{value}\n")).collect::<String>().into_bytes()
    }

    #[test]
    fn capture_accepts_export_fields_and_preserves_the_original_bytes() {
        for privacy in [PRIVACY, LEGACY_PRIVACY, "Review raw data before sharing."] {
            let mut metadata = header();
            metadata["privacy"] = privacy.into();
            let mut values = vec![metadata];
            for (step, label) in STEPS.iter().enumerate() {
                values.push(serde_json::json!({"kind":"step", "elapsed_ms":0, "step":step, "label":label}));
            }
            for api in ["input", "get-input", "get-feature", "set-feature", "set-output", "write"] {
                let mut record = transfer();
                record["api"] = api.into();
                values.push(record);
            }
            values.push(serde_json::json!({"kind":"transfer", "elapsed_ms":100, "api":"input-timeout",
                "success":false, "hex":""}));
            values.push(end(7));
            let bytes = jsonl(&values);
            let attachment = capture(&bytes).unwrap();
            assert_eq!(attachment.name, "raw.jsonl");
            assert_eq!(attachment.mime_type, "application/x-ndjson");
            assert_eq!(STANDARD.decode(&attachment.data_base64).unwrap(), bytes);
            assert!(validate(&attachment).is_ok());
        }
    }

    #[test]
    fn capture_requires_utf8_jsonl_header_terminal_end_and_real_transfer_bytes() {
        let step = serde_json::json!({"kind":"step", "elapsed_ms":0, "step":0, "label":"Normal use"});
        let mut empty = transfer();
        empty["hex"] = "".into();
        for bytes in [Vec::new(), vec![0xff, 0xfe], b"{}\n".to_vec(),
            jsonl(&[header(), end(0)]), jsonl(&[header(), step, end(0)]),
            jsonl(&[header(), empty, end(1)]), jsonl(&[header(), transfer()]),
            jsonl(&[transfer(), end(1)]), jsonl(&[header(), transfer(), end(0)]),
            jsonl(&[header(), transfer(), end(2)]), jsonl(&[header(), transfer(), end(1), end(1)]),
            jsonl(&[header(), transfer(), end(1), transfer()])] {
            assert!(capture(&bytes).is_err(), "{}", String::from_utf8_lossy(&bytes));
        }
        let valid = String::from_utf8(jsonl(&[header(), transfer(), end(1)])).unwrap();
        assert!(capture(valid.trim_end().as_bytes()).is_ok());
        assert!(capture(valid.replace('\n', "\r\n").as_bytes()).is_ok());
        for raw in [format!("\u{feff}{valid}"), format!("\n{valid}"), format!("{valid}\n"),
            valid.replacen('\n', "\n\n", 1)] { assert!(capture(raw.as_bytes()).is_err()); }
    }

    #[test]
    fn capture_rejects_unknown_missing_duplicate_and_private_metadata() {
        let original = [header(), transfer(), end(1)];
        for position in 0..original.len() {
            for field in ["path", "serial", "alias", "config", "authorization", "unexpected"] {
                let mut values = original.clone();
                values[position][field] = "private-value".into();
                assert!(capture(&jsonl(&values)).is_err(), "{position}: {field}");
            }
            for field in original[position].as_object().unwrap().keys() {
                let mut values = original.clone();
                values[position].as_object_mut().unwrap().remove(field);
                assert!(capture(&jsonl(&values)).is_err(), "{position}: {field}");
            }
        }
        let raw = String::from_utf8(jsonl(&original)).unwrap();
        for duplicate in [raw.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1),
            raw.replacen("\"hex\":\"00ABFF\"", "\"hex\":\"00ABFF\",\"hex\":\"00ABFF\"", 1),
            raw.replacen("\"kind\":\"transfer\"", "\"kind\":\"transfer\",\"kind\":\"transfer\"", 1)] {
            assert!(capture(duplicate.as_bytes()).is_err());
        }
        for (field, value) in [("schema", serde_json::json!(2)), ("app", serde_json::json!("x".repeat(81))),
            ("started_unix_ms", serde_json::json!(-1)), ("started_unix_ms", serde_json::json!(9_007_199_254_740_992u64)),
            ("vendor_id", serde_json::json!(65536)), ("product_id", serde_json::json!(-1)),
            ("usage_page", serde_json::json!(65536)), ("usage", serde_json::json!(65536)),
            ("input_bytes", serde_json::json!(65536)), ("scope", serde_json::json!("x".repeat(513))),
            ("privacy", serde_json::json!("")), ("privacy", serde_json::json!("text\nmore")),
            ("privacy", serde_json::json!("x".repeat(1025)))] {
            let mut values = original.clone();
            values[0][field] = value;
            assert!(capture(&jsonl(&values)).is_err(), "{field}");
        }
        for field in ["app", "scope", "privacy"] {
            let mut values = original.clone();
            values[0][field] = " ".into();
            assert!(capture(&jsonl(&values)).is_err());
        }
    }

    #[test]
    fn capture_rejects_unexpected_records_invalid_hex_and_unsafe_timestamps() {
        for (field, value) in [("kind", serde_json::json!("other")), ("api", serde_json::json!("private/path")),
            ("success", serde_json::json!("true")), ("elapsed_ms", serde_json::json!(-1)),
            ("elapsed_ms", serde_json::json!(9_007_199_254_740_992u64)), ("hex", serde_json::json!("0")),
            ("hex", serde_json::json!("0G")), ("hex", serde_json::json!("AA BB")),
            ("hex", serde_json::json!("AA".repeat(4097)))] {
            let mut record = transfer();
            record[field] = value;
            assert!(capture(&jsonl(&[header(), record, end(1)])).is_err(), "{field}");
        }
        for step in [serde_json::json!({"kind":"step", "elapsed_ms":100, "step":5, "label":"Normal use"}),
            serde_json::json!({"kind":"step", "elapsed_ms":100, "step":0, "label":""}),
            serde_json::json!({"kind":"step", "elapsed_ms":100, "step":0, "label":"x".repeat(121)})] {
            assert!(capture(&jsonl(&[header(), transfer(), step, end(1)])).is_err());
        }
        for reason in [serde_json::json!(null), serde_json::json!(""), serde_json::json!("x".repeat(81))] {
            let mut terminal = end(1);
            terminal["reason"] = reason;
            assert!(capture(&jsonl(&[header(), transfer(), terminal])).is_err());
        }
        let mut failed = transfer();
        failed["success"] = false.into();
        assert!(capture(&jsonl(&[header(), failed, end(1)])).is_ok());
    }

    #[test]
    fn capture_accepts_relay_bounded_metadata_and_unordered_safe_timestamps() {
        for timestamp in [0, super::super::MAX_NUMBER] {
            let mut metadata = header();
            metadata["app"] = "X-Battery custom build".into();
            metadata["scope"] = " Existing device traffic ".into();
            metadata["privacy"] = " Review before sharing. ".into();
            metadata["started_unix_ms"] = timestamp.into();
            metadata["usage_page"] = 1.into();
            for field in ["input_bytes", "output_bytes", "feature_bytes"] { metadata[field] = 0.into(); }
            let step = serde_json::json!({"kind":"step", "elapsed_ms":super::super::MAX_NUMBER,
                "step":4, "label":" 重新开启外设 "});
            let mut record = transfer();
            record["elapsed_ms"] = 300_001.into();
            let mut earlier = transfer();
            earlier["elapsed_ms"] = 0.into();
            earlier["api"] = "input-timeout".into();
            let mut terminal = end(2);
            terminal["reason"] = " Application stopped recording ".into();
            let raw = jsonl(&[metadata, step, record, earlier, terminal]);
            let attachment = capture(&raw).unwrap();
            assert!(validate(&attachment).is_ok());
            assert_eq!(STANDARD.decode(attachment.data_base64).unwrap(), raw);
        }
    }

    #[test]
    fn capture_enforces_record_and_decoded_attachment_limits() {
        let mut values = vec![header()];
        values.extend(std::iter::repeat_n(transfer(), MAX_RECORDS));
        values.push(end(MAX_RECORDS));
        assert!(capture(&jsonl(&values)).is_ok());
        values.insert(1, transfer());
        *values.last_mut().unwrap() = end(MAX_RECORDS + 1);
        assert!(capture(&jsonl(&values)).is_err());
        let mut large = transfer();
        large["hex"] = "AB".repeat(4096).into();
        assert!(capture(&jsonl(&[header(), large.clone(), end(1)])).is_ok());
        let mut values = vec![header()];
        values.extend(std::iter::repeat_n(large.clone(), 60));
        values.push(end(60));
        let raw = String::from_utf8(jsonl(&values)).unwrap();
        let mut maximum = " ".repeat(MAX_CAPTURE - raw.len());
        maximum.push_str(&raw);
        assert_eq!(maximum.len(), MAX_CAPTURE);
        assert!(capture(maximum.as_bytes()).is_ok());
        assert!(validate(&capture(maximum.as_bytes()).unwrap()).is_ok());
        maximum.push(' ');
        assert!(capture(maximum.as_bytes()).is_err());
        let raw = jsonl(&[header(), transfer(), end(1)]);
        let mut values = vec![header()];
        values.extend(std::iter::repeat_n(large, 70));
        values.push(end(70));
        assert!(jsonl(&values).len() > MAX_CAPTURE);
        assert!(capture(&jsonl(&values)).is_err());
        let mut attachment = capture(&raw).unwrap();
        attachment.data_base64 = STANDARD.encode(vec![b' '; MAX_CAPTURE + 1]);
        assert!(validate(&attachment).is_err());
    }

    #[test]
    fn capture_requires_fixed_attachment_metadata_and_canonical_base64() {
        let valid = capture(&jsonl(&[header(), transfer(), end(1)])).unwrap();
        for name in ["other.jsonl", "../raw.jsonl", "C:\\raw.jsonl", "RAW.JSONL"] {
            let mut attachment = valid.clone();
            attachment.name = name.into();
            assert!(validate(&attachment).is_err());
        }
        let mut attachment = valid.clone();
        attachment.mime_type = "application/json".into();
        assert!(validate(&attachment).is_err());
        for data in [format!("{}\n", valid.data_base64), "invalid-base64".into(), STANDARD.encode(b"{}\n")] {
            attachment = valid.clone();
            attachment.data_base64 = data;
            assert!(validate(&attachment).is_err());
        }
        let mut value = serde_json::to_value(valid).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(value["mimeType"], "application/x-ndjson");
        assert!(value["dataBase64"].is_string());
        value["path"] = "C:\\raw.jsonl".into();
        assert!(serde_json::from_value::<Capture>(value).is_err());
    }
}
