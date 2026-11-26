use crate::hid::{EnumerateReport, HidInterface, Skipped};
use std::fmt::Write as _;

const MAX_REPORT_BYTES: usize = 10 * 1024;
const MAX_PRODUCT_BYTES: usize = 1024;
const MAX_PRODUCT_CHARS: usize = 96;

pub fn collect() -> String {
    let collected_at = crate::diagnostics::unix_millis();
    let result = crate::win32::HidApi::load()
        .map_err(|reason| CollectionFailure::new(Stage::Load, &reason))
        .and_then(|api| crate::hid::enumerate_all_report(&api)
            .map_err(|reason| CollectionFailure::new(Stage::Enumeration, &reason)));
    render(collected_at, result, crate::diagnostics::anonymous_id)
}

#[derive(Clone, Copy)]
enum Stage { Load, Enumeration }

struct CollectionFailure {
    stage: Stage,
    failure: Failure,
}

impl CollectionFailure {
    fn new(stage: Stage, reason: &str) -> Self {
        Self { stage, failure: Failure::from_reason(reason) }
    }
}

struct Failure {
    category: &'static str,
    os_error: Option<u32>,
    hid_status: Option<i32>,
}

impl Failure {
    fn from_reason(reason: &str) -> Self {
        let category = if reason.starts_with("CreateFileW 失败（") { "open_failed" }
            else if reason.starts_with("HidD_GetAttributes 失败（") { "attributes_failed" }
            else if reason.starts_with("HidD_GetPreparsedData 失败（") { "descriptor_failed" }
            else if reason.starts_with("HidP_GetCaps 返回 ") { "capabilities_failed" }
            else if reason.starts_with("SetupDiGetClassDevsW 失败（") { "interface_list_failed" }
            else if reason.starts_with("SetupDiEnumDeviceInterfaces 失败（") { "enumeration_failed" }
            else if reason.starts_with("SetupDiGetDeviceInterfaceDetailW ") { "interface_detail_failed" }
            else if reason.starts_with("无法加载 hid.dll（") { "hid_library_load_failed" }
            else if reason.starts_with("hid.dll 缺少导出函数 ") { "hid_symbol_missing" }
            else { "other" };
        let hid_status = reason.strip_prefix("HidP_GetCaps 返回 ")
            .and_then(|value| value.parse::<i32>().ok());
        Self { category, os_error: system_error(reason), hid_status }
    }

    fn render(&self) -> String {
        let mut output = format!("reason={}", self.category);
        if let Some(code) = self.os_error { let _ = write!(output, "; os_error={code}"); }
        if let Some(code) = self.hid_status { let _ = write!(output, "; hid_status={code}"); }
        output
    }
}

fn system_error(reason: &str) -> Option<u32> {
    for marker in ["GetLastError=", "os error ", "os_error="] {
        for (offset, _) in reason.match_indices(marker) {
            if reason[..offset].chars().next_back()
                .is_some_and(|character| character.is_alphanumeric() || character == '_') {
                continue;
            }
            let suffix = &reason[offset + marker.len()..];
            let suffix = suffix.strip_prefix("Some(").unwrap_or(suffix);
            let digits = suffix.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || digits > 10 { continue; }
            if suffix[digits..].chars().next().is_some_and(|character|
                !character.is_whitespace() && !matches!(character, ')' | '）' | ']' | '}' | ';' | ',')) {
                continue;
            }
            if let Ok(code) = suffix[..digits].parse() { return Some(code); }
        }
    }
    None
}

fn path_ids(value: &str) -> (Option<u16>, Option<u16>) {
    let lower = value.to_ascii_lowercase();
    let Some(remainder) = lower.strip_prefix(r"\\?\hid#") else { return (None, None); };
    let Some((hardware, _)) = remainder.split_once('#') else { return (None, None); };
    (hardware_id(hardware, "vid_"), hardware_id(hardware, "pid_"))
}

fn hardware_id(hardware: &str, prefix: &str) -> Option<u16> {
    let mut fields = hardware.split('&').filter_map(|field| field.strip_prefix(prefix));
    let value = fields.next()?;
    if fields.next().is_some() || value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(value, 16).ok()
}

fn private_name(value: &str) -> bool {
    let normalized: String = value.chars()
        .filter(|character| !character.is_whitespace())
        .map(|character| {
            if ('\u{ff01}'..='\u{ff5e}').contains(&character) {
                char::from_u32(character as u32 - 0xfee0).unwrap_or(character)
            } else { character }
        })
        .flat_map(char::to_lowercase)
        .collect();
    normalized.chars().any(|character| matches!(character, '\\' | '/' | '@' | '`'))
        || ["serial:", "serial=", "serialnumber:", "serialnumber=", "serialno:", "serialno=",
            "path:", "path=", "alias:", "alias=", "config:", "config=",
            "authorization:", "authorization=", "bearer", "token:", "token=",
            "password:", "password=", "passwd:", "passwd=", "secret:", "secret=",
            "credential:", "credential=", "credentials:", "credentials=",
            "account:", "account=", "username:", "username=", "user:", "user=",
            "api_key:", "api_key=", "apikey:", "apikey=", "access_key:", "access_key=",
            "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"]
            .iter().any(|marker| normalized.contains(marker))
}

fn product_name(value: &str) -> String {
    if value.len() > MAX_PRODUCT_BYTES { return "omitted_unsafe".into(); }
    if value.chars().any(|character| matches!(character,
        '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')) {
        return "omitted_unsafe".into();
    }
    let clean: String = value.chars().filter(|character| !character.is_control()).collect();
    let clean = clean.trim();
    if clean.is_empty() { return "unavailable".into(); }
    if private_name(clean) { return "omitted_unsafe".into(); }
    let bounded: String = clean.chars().take(MAX_PRODUCT_CHARS).collect();
    let quoted = format!("{bounded:?}");
    if quoted.contains(":\\") { return "omitted_unsafe".into(); }
    quoted
}

struct Row {
    order: String,
    channel: u64,
    compact: String,
    details: String,
}

impl Row {
    fn from_interface(interface: &HidInterface, channel_id: &impl Fn(&str) -> u64) -> Self {
        let order = interface.path.to_ascii_lowercase();
        let channel = channel_id(&order);
        let compact = format!("result=ok; VID/PID={:04X}:{:04X}", interface.vendor_id, interface.product_id);
        let details = format!(
            "{compact}; Usage={:04X}:{:04X}; input/output/feature={}/{}/{}; product={}",
            interface.usage_page, interface.usage,
            interface.input_len, interface.output_len, interface.feature_len, product_name(&interface.product));
        Self { order, channel, compact, details }
    }

    fn from_skipped(skipped: &Skipped, channel_id: &impl Fn(&str) -> u64) -> Self {
        let order = skipped.path.to_ascii_lowercase();
        let channel = channel_id(&order);
        let (vendor, product) = path_ids(&skipped.path);
        let hex = |value: Option<u16>| value.map_or_else(|| "unknown".into(), |value| format!("{value:04X}"));
        let compact = format!("result=failed; VID/PID={}:{}", hex(vendor), hex(product));
        let details = format!(
            "{compact}; id_source={}; Usage=unknown; input/output/feature=unknown; product=unavailable; {}",
            if vendor.is_some() || product.is_some() { "interface_identifier" } else { "unknown" },
            Failure::from_reason(&skipped.reason).render());
        Self { order, channel, compact, details }
    }

    fn line(&self, index: usize, compact: bool) -> String {
        let details = if compact { &self.compact } else { &self.details };
        format!("interface={:04}; channel={:016x}; {details}\n", index + 1, self.channel)
    }
}

fn footer(listed: usize, omitted: usize, compact: bool) -> String {
    let (detailed_count, compact_count) = if compact { (0, listed) } else { (listed, 0) };
    format!("metadata_compact={compact}; detailed_count={detailed_count}; compact_count={compact_count}\nlisted_count={listed}; omitted_count={omitted}; truncated={}\n", omitted != 0)
}

fn render(collected_at: u128, result: Result<EnumerateReport, CollectionFailure>, channel_id: impl Fn(&str) -> u64) -> String {
    let mut output = format!(
        "HID interface inventory\ncollected_at_unix_ms={collected_at} (UTC)\nscope=all_present_hid_interfaces; report_budget_bytes={MAX_REPORT_BYTES}\nInterface numbers apply only to this snapshot. Channels identify interfaces within this process, not physical devices.\n");
    let report = match result {
        Ok(report) => report,
        Err(failure) => {
            let stage = match failure.stage { Stage::Load => "api_load", Stage::Enumeration => "enumeration" };
            let _ = writeln!(output, "enumeration=failed; stage={stage}; {}", failure.failure.render());
            output.push_str("interface_count=unknown; success_count=unknown; failed_count=unknown; enumeration_failure_count=1\nmetadata_compact=false; detailed_count=0; compact_count=0\nlisted_count=0; omitted_count=unknown; truncated=false\n");
            return output;
        }
    };
    let success_count = report.interfaces.len();
    let failed_count = report.skipped.len();
    let total = success_count + failed_count;
    let _ = writeln!(output, "enumeration=complete; enumeration_failure_count=0\ninterface_count={total}; success_count={success_count}; failed_count={failed_count}");
    let mut rows: Vec<_> = report.interfaces.iter().map(|interface| Row::from_interface(interface, &channel_id))
        .chain(report.skipped.iter().map(|skipped| Row::from_skipped(skipped, &channel_id))).collect();
    rows.sort_by(|left, right| left.order.cmp(&right.order).then_with(|| left.details.cmp(&right.details)));
    let full_size = rows.iter().enumerate().fold(output.len() + footer(total, 0, false).len(),
        |bytes, (index, row)| bytes.saturating_add(row.line(index, false).len()));
    let compact = full_size > MAX_REPORT_BYTES;
    let mut listed = 0;
    for (index, row) in rows.iter().enumerate() {
        let line = row.line(index, compact);
        let footer_size = footer(listed + 1, total - listed - 1, compact).len();
        if output.len() + line.len() + footer_size > MAX_REPORT_BYTES { break; }
        output.push_str(&line);
        listed += 1;
    }
    output.push_str(&footer(listed, total - listed, compact));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLLECTED_AT: u128 = 1_795_622_400_000;

    fn interface(path: &str) -> HidInterface {
        HidInterface {
            path: path.into(), vendor_id: 0xdead, product_id: 0xbeef, usage_page: 0xff00,
            usage: 2, input_len: 19, output_len: 19, feature_len: 0,
            product: "Unadapted mouse".into(), serial: "private-serial".into(),
        }
    }

    fn channel(value: &str) -> u64 {
        value.bytes().fold(0u64, |hash, byte| hash.wrapping_mul(131).wrapping_add(u64::from(byte)))
    }

    fn inventory(interfaces: Vec<HidInterface>, skipped: Vec<Skipped>) -> String {
        render(COLLECTED_AT, Ok(EnumerateReport { interfaces, skipped }), channel)
    }

    fn assert_valid_report(report: &str) {
        let draft = crate::feedback::client::Draft {
            category: "device".into(),
            device: crate::feedback::client::Device {
                name: "Unknown mouse".into(), family: "mouse".into(), connection: "unknown".into(),
                brand: None, model: None,
            },
            description: "Device is not recognized.".into(), diagnostics: Some(report.into()), images: Vec::new(), capture: None,
        };
        assert!(crate::feedback::client::validate_details(&draft).is_ok(), "{report}");
        assert!(report.len() <= MAX_REPORT_BYTES);
    }

    #[test]
    fn unadapted_interfaces_include_metadata_without_identifiers() {
        let report = inventory(vec![interface(r"\\?\hid#vid_dead&pid_beef#private-account")], Vec::new());
        for expected in ["collected_at_unix_ms=1795622400000 (UTC)", "success_count=1; failed_count=0",
            "interface=0001; channel=", "VID/PID=DEAD:BEEF", "Usage=FF00:0002",
            "input/output/feature=19/19/0", "product=\"Unadapted mouse\"", "omitted_count=0; truncated=false",
            "metadata_compact=false; detailed_count=1; compact_count=0"] {
            assert!(report.contains(expected), "{expected}");
        }
        for private in ["private-account", "private-serial", r"\\?\", "vid_dead"] {
            assert!(!report.contains(private), "{private}");
        }
        assert_valid_report(&report);
    }

    #[test]
    fn identical_models_and_multiple_collections_keep_separate_interfaces() {
        let paths = [
            r"\\?\hid#vid_dead&pid_beef&col01#receiver-a",
            r"\\?\hid#vid_dead&pid_beef&col02#receiver-a",
            r"\\?\hid#vid_dead&pid_beef&col01#receiver-b",
        ];
        let report = inventory(paths.iter().map(|path| interface(path)).collect(), Vec::new());
        assert_eq!(report.matches("VID/PID=DEAD:BEEF").count(), 3);
        assert!(report.contains("interface_count=3; success_count=3; failed_count=0"));
        for (index, path) in paths.iter().enumerate() {
            assert!(report.contains(&format!("channel={:016x}", channel(path))));
            assert!(report.contains(&format!("interface={:04};", index + 1)));
        }
        assert!(report.contains("not physical devices"));
        assert_valid_report(&report);
    }

    #[test]
    fn order_and_numbering_are_stable_and_channels_use_lowercase_paths() {
        let mut interfaces = vec![interface("INTERFACE-Z"), interface("interface-a")];
        let mut skipped = vec![
            Skipped { path: "INTERFACE-Y".into(), reason: "unknown".into() },
            Skipped { path: "interface-b".into(), reason: "unknown".into() },
        ];
        let first = inventory(interfaces.clone(), skipped.clone());
        interfaces.reverse();
        skipped.reverse();
        let second = render(COLLECTED_AT, Ok(EnumerateReport { interfaces, skipped }), |path| {
            assert_eq!(path, path.to_ascii_lowercase());
            channel(path)
        });
        assert_eq!(first, second);
        for (index, path) in ["interface-a", "interface-b", "interface-y", "interface-z"].iter().enumerate() {
            assert!(first.contains(&format!("interface={:04}; channel={:016x}", index + 1, channel(path))));
        }
    }

    #[test]
    fn failed_interfaces_keep_safe_ids_categories_and_numeric_codes() {
        let report = inventory(Vec::new(), vec![Skipped {
            path: r"\\?\HID#VID_24aE&PID_1415&MI_00#private-account".into(),
            reason: "CreateFileW 失败（GetLastError=5） C:\\Users\\Alice; serial=private-serial; bearer private-token".into(),
        }, Skipped {
            path: r"\\?\hid#vid_0b05&pid_1ace#private-instance".into(),
            reason: "HidP_GetCaps 返回 -1072627711".into(),
        }]);
        for expected in ["success_count=0; failed_count=2", "result=failed; VID/PID=24AE:1415",
            "id_source=interface_identifier", "reason=open_failed; os_error=5",
            "reason=capabilities_failed; hid_status=-1072627711", "Usage=unknown", "omitted_count=0"] {
            assert!(report.contains(expected), "{expected}");
        }
        for private in ["Alice", "private-", "GetLastError", "CreateFileW", "bearer", "serial="] {
            assert!(!report.contains(private), "{private}");
        }
        assert_valid_report(&report);
    }

    #[test]
    fn path_ids_require_exact_unique_fields_in_the_hardware_component() {
        assert_eq!(path_ids(r"\\?\HID#VID_0B05&PID_1ace&col01#instance"), (Some(0x0b05), Some(0x1ace)));
        for path in [r"\\?\hid#vid_12345&pid_abcdx#instance", r"\\?\hid#vid_zzzz&pid_#instance",
            r"\\?\hid#xvid_1234&xpid_5678#instance", r"\\?\hid#unknown#vid_1234&pid_5678",
            r"\\?\hid#vid_1234&vid_1234&pid_5678&pid_5678#instance", "vid_1234&pid_5678",
            r"C:\private\vid_1234&pid_5678", r"\\?\hid#vid_1234&pid_5678"] {
            assert_eq!(path_ids(path), (None, None), "{path}");
        }
        assert_eq!(path_ids(r"\\?\hid#vid_1234&pid_invalid#instance"), (Some(0x1234), None));
    }

    #[test]
    fn system_codes_are_bounded_numbers_without_partial_parses() {
        for (reason, expected) in [("error (os error 32)", 32), ("os_error=Some(17)", 17),
            ("失败（GetLastError=5）", 5), ("os_error=4294967295", u32::MAX)] {
            assert_eq!(system_error(reason), Some(expected));
        }
        for reason in ["os_error=-5", "os_error=4294967296", "os_error=12345678901",
            "os_error=5secret", "os_error=0x05", "prefixos_error=5", "GetLastError=5.2",
            "os_error=None", "os_error=", "os_error=５"] {
            assert_eq!(system_error(reason), None, "{reason}");
        }
        assert_eq!(Failure::from_reason("private-username: token=private-token (os error 5)").render(), "reason=other; os_error=5");
        assert_eq!(Failure::from_reason("HidP_GetCaps 返回 -1 private-data").render(), "reason=capabilities_failed");
    }

    #[test]
    fn product_names_preserve_unicode_and_bound_display_length() {
        assert_eq!(product_name("  未适配鼠标\t Pro\n  "), "\"未适配鼠标 Pro\"");
        assert_eq!(product_name("\0\r\n\t "), "unavailable");
        assert_eq!(product_name("Mouse \"Edition\""), "\"Mouse \\\"Edition\\\"\"");
        assert_eq!(product_name(&"鼠".repeat(100)), format!("\"{}\"", "鼠".repeat(MAX_PRODUCT_CHARS)));
        assert_eq!(product_name(&"x".repeat(MAX_PRODUCT_BYTES + 1)), "omitted_unsafe");
    }

    #[test]
    fn unsafe_names_cannot_smuggle_private_markers_or_break_validation() {
        let unsafe_names = [r"C:\Users\Alice\mouse", r"\\?\hid#private", "/home/alice/mouse",
            "serial:private", "Ser\0ial=private", "serial : private", "path=private", "alias:private",
            "config=private", "Authorization:private", "BeArEr private", "bear\ner private",
            "token=private", "password:private", "account=private", "user@example.com",
            "github_pat_private", "Ｐａｔｈ：private", "ｓｅｒｉａｌ＝private", "Ｃ：＼Users＼Alice",
            "bear\u{200b}er private", "Mouse\u{202e}private", "```\nprivate\n```"];
        for name in unsafe_names {
            let mut device = interface("private-interface");
            device.product = name.into();
            let report = inventory(vec![device], Vec::new());
            assert!(report.contains("product=omitted_unsafe"), "{name}");
            assert!(!report.contains("private"), "{name}");
            assert_valid_report(&report);
        }
        assert_eq!(product_name(&format!("{} serial:private", "x".repeat(MAX_PRODUCT_CHARS))), "omitted_unsafe");
    }

    #[test]
    fn missing_metadata_and_unknown_errors_are_reported_without_raw_text() {
        let mut device = interface("private-interface");
        device.product.clear();
        let report = inventory(vec![device], vec![Skipped {
            path: "private-unparseable-path".into(), reason: "private error with account and settings".into(),
        }]);
        assert!(report.contains("product=unavailable"));
        assert!(report.contains("VID/PID=unknown:unknown; id_source=unknown"));
        assert!(report.contains("reason=other"));
        assert!(!report.contains("private"));
        assert_valid_report(&report);
    }

    #[test]
    fn detailed_overflow_keeps_every_interface_id_and_result_in_compact_rows() {
        let interfaces: Vec<_> = (0..80).map(|index| {
            let mut device = interface(&format!(r"\\?\hid#vid_dead&pid_{index:04x}#interface-{index:04}"));
            device.product_id = index;
            device.product = "鼠".repeat(MAX_PRODUCT_CHARS);
            device
        }).collect();
        let skipped: Vec<_> = (80..100).map(|index| Skipped {
            path: format!(r"\\?\hid#vid_dead&pid_{index:04x}#interface-{index:04}"),
            reason: "CreateFileW 失败（GetLastError=32）".into(),
        }).collect();
        assert!(interfaces.iter().map(|device| device.product.len()).sum::<usize>() > MAX_REPORT_BYTES);
        let report = inventory(interfaces.clone(), skipped.clone());
        assert!(report.contains("interface_count=100; success_count=80; failed_count=20"));
        assert!(report.contains("metadata_compact=true; detailed_count=0; compact_count=100"));
        assert!(report.ends_with("listed_count=100; omitted_count=0; truncated=false\n"));
        let lines: Vec<_> = report.lines().filter(|line| line.starts_with("interface=")).collect();
        assert_eq!(lines.len(), 100);
        for (index, path) in interfaces.iter().map(|device| device.path.as_str())
            .chain(skipped.iter().map(|failure| failure.path.as_str())).enumerate() {
            let result = if index < 80 { "ok" } else { "failed" };
            assert_eq!(lines[index], format!(
                "interface={:04}; channel={:016x}; result={result}; VID/PID=DEAD:{index:04X}",
                index + 1, channel(&path.to_ascii_lowercase())));
        }
        assert_valid_report(&report);
    }

    #[test]
    fn only_compact_overflow_omits_interfaces_with_exact_counts() {
        let interfaces = (0..300).map(|index| {
            let mut device = interface(&format!("interface-{index:04}"));
            device.product = "鼠".repeat(MAX_PRODUCT_CHARS);
            device
        }).collect();
        let skipped = (300..400).map(|index| Skipped {
            path: format!("interface-{index:04}"), reason: "CreateFileW 失败（GetLastError=32）".into(),
        }).collect();
        let report = inventory(interfaces, skipped);
        let listed = report.lines().filter(|line| line.starts_with("interface=")).count();
        assert!(listed > 0 && listed < 400);
        assert!(report.contains("interface_count=400; success_count=300; failed_count=100"));
        assert!(report.ends_with(&format!("listed_count={listed}; omitted_count={}; truncated=true\n", 400 - listed)));
        assert!(report.contains(&format!("metadata_compact=true; detailed_count=0; compact_count={listed}")));
        assert!(report.lines().filter(|line| line.starts_with("interface=")).all(|line| line.ends_with("VID/PID=DEAD:BEEF")));
        let next = format!("interface={:04}; channel={:016x}; result=ok; VID/PID=DEAD:BEEF\n",
            listed + 1, channel(&format!("interface-{listed:04}")));
        let with_next_size = report.len() - footer(listed, 400 - listed, true).len()
            + next.len() + footer(listed + 1, 399 - listed, true).len();
        assert!(with_next_size > MAX_REPORT_BYTES);
        assert_valid_report(&report);
    }

    #[test]
    fn empty_enumerations_are_distinct_from_interface_and_collection_failures() {
        let empty = inventory(Vec::new(), Vec::new());
        assert!(empty.contains("enumeration=complete"));
        assert!(empty.contains("interface_count=0; success_count=0; failed_count=0"));
        assert!(empty.ends_with("listed_count=0; omitted_count=0; truncated=false\n"));
        let failed_interface = inventory(Vec::new(), vec![Skipped { path: "interface".into(), reason: "unknown".into() }]);
        assert!(failed_interface.contains("interface_count=1; success_count=0; failed_count=1"));
        assert!(failed_interface.contains("listed_count=1; omitted_count=0"));
        for (stage, reason, expected) in [
            (Stage::Load, "无法加载 hid.dll（GetLastError=126） private-detail", "stage=api_load; reason=hid_library_load_failed; os_error=126"),
            (Stage::Enumeration, "SetupDiEnumDeviceInterfaces 失败（GetLastError=31） private-detail", "stage=enumeration; reason=enumeration_failed; os_error=31"),
        ] {
            let report = render(COLLECTED_AT, Err(CollectionFailure::new(stage, reason)), channel);
            assert!(report.contains("enumeration=failed"));
            assert!(report.contains(expected));
            assert!(report.contains("interface_count=unknown; success_count=unknown; failed_count=unknown; enumeration_failure_count=1"));
            assert!(report.ends_with("listed_count=0; omitted_count=unknown; truncated=false\n"));
            assert!(!report.contains("private-detail"));
            assert_valid_report(&report);
        }
        assert_valid_report(&empty);
        assert_valid_report(&failed_interface);
    }
}
