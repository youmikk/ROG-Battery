use crate::hid::{path_ids, EnumerateReport, GroupedEnumerateReport, HidInterface, Skipped};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const MAX_REPORT_BYTES: usize = 10 * 1024;
const MAX_PRODUCT_BYTES: usize = 1024;
const MAX_PRODUCT_CHARS: usize = 96;

#[derive(Clone)]
pub struct UnknownDevice {
    pub name: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub interfaces: usize,
    pub diagnostics: String,
}

#[derive(Clone)]
pub struct DeviceInventory {
    pub report: String,
    pub unknown: Vec<UnknownDevice>,
}

pub fn collect_for_devices(devices: &[super::Candidate]) -> DeviceInventory {
    let collected_at = crate::diagnostics::unix_millis();
    let result = crate::win32::HidApi::load()
        .map_err(|reason| CollectionFailure::new(Stage::Load, &reason))
        .and_then(|api| crate::hid::enumerate_all_grouped_report(&api)
            .map_err(|reason| CollectionFailure::new(Stage::Enumeration, &reason)));
    inventory_for_devices(collected_at, result, devices, crate::diagnostics::anonymous_id)
}

fn inventory_for_devices(
    collected_at: u128,
    result: Result<GroupedEnumerateReport, CollectionFailure>,
    devices: &[super::Candidate],
    channel_id: impl Fn(&str) -> u64,
) -> DeviceInventory {
    match result {
        Ok(grouped) => {
            let unknown = unknown_devices(collected_at, &grouped, devices, &channel_id);
            DeviceInventory { report: render(collected_at, Ok(grouped.report), channel_id), unknown }
        }
        Err(failure) => DeviceInventory {
            report: render(collected_at, Err(failure), channel_id), unknown: Vec::new(),
        },
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey {
    Physical(String),
    Collection(String),
    Path(String),
}

fn collection_group(path: &str) -> String {
    let path = path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('#').collect();
    let [prefix, hardware, instance, class] = parts.as_slice() else { return path; };
    if *prefix != r"\\?\hid" || instance.is_empty() || class.is_empty() { return path; }
    let mut collections = hardware.split('&').filter_map(|field| field.strip_prefix("col"));
    let Some(collection) = collections.next() else { return path; };
    if collections.next().is_some() || collection.len() != 2
        || !collection.bytes().all(|byte| byte.is_ascii_hexdigit()) { return path; }
    let number = u8::from_str_radix(collection, 16).unwrap();
    let one_based = format!("&00{collection}");
    let zero_based = number.checked_sub(1).map(|number| format!("&{number:04x}"));
    let suffix = if instance.ends_with(&one_based) { one_based.as_str() }
        else if let Some(suffix) = zero_based.as_deref().filter(|suffix| instance.ends_with(*suffix)) { suffix }
        else { return path; };
    let instance = &instance[..instance.len() - suffix.len()];
    if instance.is_empty() { return path; }
    let field = format!("col{collection}");
    let hardware = hardware.split('&').filter(|value| *value != field).collect::<Vec<_>>().join("&");
    format!("{prefix}#{hardware}#{instance}#{class}")
}

fn group_key(path: &str, physical_groups: &BTreeMap<String, String>,
    collections: &BTreeMap<String, BTreeSet<String>>) -> GroupKey {
    let path = path.to_ascii_lowercase();
    if let Some(group) = physical_groups.get(&path) {
        return GroupKey::Physical(group.clone());
    }
    let collection = collection_group(&path);
    match collections.get(&collection) {
        Some(groups) if groups.len() == 1 => GroupKey::Physical(groups.first().unwrap().clone()),
        Some(_) => GroupKey::Path(path),
        None => GroupKey::Collection(collection),
    }
}

fn unique_id(mut values: impl Iterator<Item = u16>) -> Option<u16> {
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn unknown_devices(collected_at: u128, grouped: &GroupedEnumerateReport,
    devices: &[super::Candidate], channel_id: &impl Fn(&str) -> u64) -> Vec<UnknownDevice> {
    let mut collections = BTreeMap::<String, BTreeSet<String>>::new();
    for (path, physical) in &grouped.physical_groups {
        collections.entry(collection_group(path)).or_default().insert(physical.clone());
    }
    let known: BTreeSet<_> = devices.iter().filter_map(|device| device.interface.as_ref())
        .map(|interface| group_key(&interface.path, &grouped.physical_groups, &collections)).collect();
    let mut groups = BTreeMap::<GroupKey, EnumerateReport>::new();
    for interface in &grouped.report.interfaces {
        let key = group_key(&interface.path, &grouped.physical_groups, &collections);
        if known.contains(&key) { continue; }
        groups.entry(key).or_insert_with(|| EnumerateReport { interfaces: Vec::new(), skipped: Vec::new() })
            .interfaces.push(interface.clone());
    }
    for skipped in &grouped.report.skipped {
        let key = group_key(&skipped.path, &grouped.physical_groups, &collections);
        if known.contains(&key) { continue; }
        groups.entry(key).or_insert_with(|| EnumerateReport { interfaces: Vec::new(), skipped: Vec::new() })
            .skipped.push(skipped.clone());
    }
    let system = super::system_version();
    groups.into_values().map(|report| {
        let name = report.interfaces.iter().filter_map(|interface| safe_product_name(&interface.product).ok())
            .min().unwrap_or_default();
        let ids: Vec<_> = report.interfaces.iter().map(|interface| (Some(interface.vendor_id), Some(interface.product_id)))
            .chain(report.skipped.iter().map(|skipped| path_ids(&skipped.path))).collect();
        let vendor_id = unique_id(ids.iter().filter_map(|ids| ids.0));
        let product_id = unique_id(ids.iter().filter_map(|ids| ids.1));
        let interfaces = report.interfaces.len() + report.skipped.len();
        let hex = |value: Option<u16>| value.map_or_else(|| "unknown".into(), |value| format!("{value:04X}"));
        let diagnostics = format!(
            "App: X-Battery {}\nSystem: {system}\nModel: {}\nVID/PID: {}:{}\nDevice selection: system-enumerated; identity=unconfirmed; support=unknown\n\nSelected interface inventory\n{}",
            crate::gui::APP_VERSION, if name.is_empty() { "unknown" } else { &name }, hex(vendor_id), hex(product_id),
            render(collected_at, Ok(report), channel_id));
        UnknownDevice { name, vendor_id, product_id, interfaces, diagnostics }
    }).collect()
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

fn safe_product_name(value: &str) -> Result<String, &'static str> {
    if value.len() > MAX_PRODUCT_BYTES { return Err("omitted_unsafe"); }
    if value.chars().any(|character| matches!(character,
        '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')) {
        return Err("omitted_unsafe");
    }
    let clean: String = value.chars().filter(|character| !character.is_control()).collect();
    let clean = clean.trim();
    if clean.is_empty() { return Err("unavailable"); }
    if private_name(clean) { return Err("omitted_unsafe"); }
    let bounded: String = clean.chars().take(MAX_PRODUCT_CHARS).collect();
    let quoted = format!("{bounded:?}");
    if quoted.contains(":\\") { return Err("omitted_unsafe"); }
    Ok(bounded)
}

fn product_name(value: &str) -> String {
    safe_product_name(value).map(|value| format!("{value:?}")).unwrap_or_else(str::to_string)
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

    fn path(instance: &str, usb_interface: u8, collection: u8) -> String {
        format!(r"\\?\hid#vid_dead&pid_beef&mi_{usb_interface:02x}&col{collection:02x}#7&{instance}&0&{:04x}#{{hid}}",
            collection - 1)
    }

    fn grouped(interfaces: Vec<HidInterface>, skipped: Vec<Skipped>, groups: &[(&str, &str)]) -> GroupedEnumerateReport {
        GroupedEnumerateReport {
            report: EnumerateReport { interfaces, skipped },
            physical_groups: groups.iter().map(|(path, group)| (path.to_ascii_lowercase(), group.to_ascii_lowercase())).collect(),
        }
    }

    fn known_device(interface: Option<&HidInterface>) -> crate::feedback::Candidate {
        crate::feedback::Candidate {
            key: 0,
            interface: interface.cloned(),
            capture_interface: interface.filter(|interface| crate::capture::eligible(interface)).cloned(),
            label: "private-alias".into(), battery: "—".into(), diagnostics: "private-known-report".into(),
            device: crate::feedback::client::Device {
                name: "Known device".into(), family: "mouse".into(), connection: "unknown".into(),
                brand: None, model: None,
            },
        }
    }

    #[test]
    fn physical_groups_join_cross_interface_siblings_without_merging_identical_devices() {
        let first = interface(&path("first", 0, 1));
        let sibling = interface(&path("first", 1, 2));
        let second = interface(&path("second", 0, 1));
        let groups = [(&*first.path, "usb\\vid_dead&pid_beef\\first"),
            (&*sibling.path, "usb\\vid_dead&pid_beef\\first"),
            (&*second.path, "usb\\vid_dead&pid_beef\\second")];
        let mut report = grouped(vec![first.clone(), sibling.clone(), second.clone()], Vec::new(), &groups);
        let expected = render(COLLECTED_AT, Ok(report.report.clone()), channel);
        let initial = unknown_devices(COLLECTED_AT, &report, &[], &channel);
        assert_eq!(initial.iter().map(|device| device.interfaces).collect::<Vec<_>>(), vec![2, 1]);
        for device in &initial {
            assert_eq!((device.vendor_id, device.product_id), (Some(0xdead), Some(0xbeef)));
            assert_eq!(device.name, first.product);
            for field in [format!("App: X-Battery {}\n", crate::gui::APP_VERSION),
                format!("System: {}\n", crate::feedback::system_version()), format!("Model: {}\n", first.product),
                "VID/PID: DEAD:BEEF\n".into(), "Selected interface inventory\nHID interface inventory\n".into()] {
                assert!(device.diagnostics.contains(&field));
            }
        }
        report.report.interfaces.reverse();
        let inventory = inventory_for_devices(COLLECTED_AT, Ok(report), &[], channel);
        assert_eq!(inventory.report, expected);
        assert_eq!(inventory.unknown.iter().map(|device| &device.diagnostics).collect::<Vec<_>>(),
            initial.iter().map(|device| &device.diagnostics).collect::<Vec<_>>());
    }

    #[test]
    fn collection_fallback_keeps_usb_interfaces_and_instances_separate() {
        let first = path("first", 0, 1);
        let sibling = path("first", 0, 2);
        let another_interface = path("first", 1, 1);
        let second = path("second", 0, 1);
        let malformed = first.replace("&0000#", "&unexpected#");
        assert_eq!(collection_group(&first), collection_group(&sibling.to_ascii_uppercase()));
        assert_ne!(collection_group(&first), collection_group(&another_interface));
        assert_ne!(collection_group(&first), collection_group(&second));
        assert_eq!(collection_group(&malformed), malformed);
        let one_based_first = first.replace("&0000#", "&0001#");
        let one_based_sibling = sibling.replace("&0001#", "&0002#");
        assert_eq!(collection_group(&one_based_first), collection_group(&one_based_sibling));
        let report = grouped([first, sibling, another_interface, second, malformed]
            .iter().map(|path| interface(path)).collect(), Vec::new(), &[]);
        let unknown = unknown_devices(COLLECTED_AT, &report, &[], &channel);
        let mut counts: Vec<_> = unknown.iter().map(|device| device.interfaces).collect();
        counts.sort_unstable();
        assert_eq!(counts, vec![1, 1, 1, 2]);
    }

    #[test]
    fn known_non_capture_interface_excludes_its_physical_group_and_preserves_known_slots() {
        let mut first = interface(&path("first", 0, 1));
        first.usage_page = 1;
        first.usage = 2;
        let sibling = interface(&path("first", 1, 1));
        let failed_path = path("first", 2, 1);
        let second = interface(&path("second", 0, 1));
        let devices = vec![known_device(Some(&first)), known_device(Some(&first)), known_device(None)];
        assert!(devices[0].capture_interface.is_none());
        let report = grouped(vec![first.clone(), sibling.clone(), second.clone()],
            vec![Skipped { path: failed_path.clone(), reason: "occupied".into() }],
            &[(&first.path, "first"), (&sibling.path, "first"), (&failed_path, "first"), (&second.path, "second")]);
        let inventory = inventory_for_devices(COLLECTED_AT, Ok(report), &devices, channel);
        assert_eq!(inventory.unknown.len(), 1);
        assert_eq!(inventory.unknown[0].interfaces, 1);
        assert!(inventory.unknown[0].diagnostics.contains(&format!("channel={:016x}", channel(&second.path))));
        assert!(inventory.report.contains("interface_count=4; success_count=3; failed_count=1"));
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].interface.as_ref().unwrap().path, first.path);
        assert_eq!(devices[1].label, "private-alias");
    }

    #[test]
    fn known_collection_siblings_are_excluded_even_with_partial_parent_evidence() {
        let known = interface(&path("first", 0, 1).to_ascii_uppercase());
        let sibling = interface(&path("first", 0, 2));
        let another_interface = interface(&path("first", 1, 1));
        let second = interface(&path("second", 0, 1));
        let failed = Skipped { path: path("first", 0, 3), reason: "occupied".into() };
        let devices = [known_device(Some(&known))];
        let fallback = grouped(vec![sibling.clone(), another_interface.clone(), second.clone()], vec![failed.clone()], &[]);
        assert_eq!(unknown_devices(COLLECTED_AT, &fallback, &devices, &channel).len(), 2);
        let resolved = grouped(vec![sibling.clone(), another_interface.clone(), second.clone()], vec![failed],
            &[(&sibling.path, "first"), (&another_interface.path, "first"), (&second.path, "second")]);
        let unknown = unknown_devices(COLLECTED_AT, &resolved, &devices, &channel);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].interfaces, 1);
    }

    #[test]
    fn contradictory_physical_evidence_does_not_merge_or_hide_other_instances() {
        let first = interface(&path("shared", 0, 1));
        let second = interface(&path("shared", 0, 2));
        let uncertain = interface(&path("shared", 0, 3));
        let devices = [known_device(Some(&first))];
        let report = grouped(vec![first.clone(), second.clone(), uncertain], Vec::new(),
            &[(&first.path, "physical-first"), (&second.path, "physical-second")]);
        let unknown = unknown_devices(COLLECTED_AT, &report, &devices, &channel);
        assert_eq!(unknown.len(), 2);
        assert!(unknown.iter().all(|device| device.interfaces == 1));
    }

    #[test]
    fn failed_interfaces_remain_selectable_with_partial_ids_and_safe_failure_details() {
        let first = path("first", 0, 1);
        let sibling = path("first", 1, 1);
        let second = path("second", 0, 1);
        let partial = r"\\?\hid#vid_1234&pid_invalid#private-partial";
        let report = grouped(Vec::new(), [&*first, &*sibling, &*second, partial, "private-path"].iter()
            .map(|path| Skipped { path: (*path).into(),
                reason: "CreateFileW 失败（GetLastError=32） C:\\Users\\Alice serial=private-serial".into() }).collect(),
            &[(&first, "first"), (&sibling, "first"), (&second, "second")]);
        let inventory = inventory_for_devices(COLLECTED_AT, Ok(report), &[], channel);
        assert_eq!(inventory.unknown.len(), 4);
        assert_eq!(inventory.unknown.iter().map(|device| device.interfaces).sum::<usize>(), 5);
        assert_eq!(inventory.unknown.iter().filter(|device| device.vendor_id == Some(0xdead)).count(), 2);
        assert!(inventory.unknown.iter().any(|device| device.vendor_id == Some(0x1234) && device.product_id.is_none()));
        assert!(inventory.unknown.iter().any(|device| device.vendor_id.is_none() && device.product_id.is_none()));
        for device in &inventory.unknown {
            assert!(device.name.is_empty());
            assert!(device.diagnostics.contains("Model: unknown\n"));
            assert!(device.diagnostics.contains("identity=unconfirmed; support=unknown"));
            assert!(device.diagnostics.contains("reason=open_failed; os_error=32"));
            for private in ["private-", "Alice", r"\\?\", "serial=", "unsupported"] {
                assert!(!device.diagnostics.contains(private));
            }
            assert_valid_report(&device.diagnostics);
        }
        assert!(inventory.unknown.iter().any(|device| device.diagnostics.contains("VID/PID: 1234:unknown\n")));
        assert!(inventory.unknown.iter().any(|device| device.diagnostics.contains("VID/PID: unknown:unknown\n")));
    }

    #[test]
    fn unknown_names_use_only_safe_products_and_never_paths_serials_or_known_aliases() {
        for product in ["", " \t\n", "serial=private-serial", r"C:\Users\Alice\mouse",
            "token=private-token", "Mouse\u{202e}private", "Ｐａｔｈ：private"] {
            let mut device = interface("private-path");
            device.product = product.into();
            let report = grouped(vec![device], Vec::new(), &[("private-path", "private-parent")]);
            let inventory = inventory_for_devices(COLLECTED_AT, Ok(report), &[known_device(None)], channel);
            assert_eq!(inventory.unknown.len(), 1);
            assert!(inventory.unknown[0].name.is_empty());
            for value in [&inventory.report, &inventory.unknown[0].diagnostics] {
                for private in ["private-", "Alice", "serial=", r"\\?\"] { assert!(!value.contains(private)); }
                assert_valid_report(value);
            }
        }
        let mut device = interface("private-path");
        device.product = "  鼠标 \"Pro\"\n  ".into();
        let report = grouped(vec![device], Vec::new(), &[]);
        let unknown = unknown_devices(COLLECTED_AT, &report, &[], &channel);
        assert_eq!(unknown[0].name, "鼠标 \"Pro\"");
        assert!(unknown[0].diagnostics.contains("product=\"鼠标 \\\"Pro\\\"\""));
    }

    #[test]
    fn report_truncation_does_not_remove_unknown_choices_and_scan_failures_do_not_invent_them() {
        let report = grouped((0..180).map(|index| interface(&format!("private-path-{index:04}"))).collect(), Vec::new(), &[]);
        let inventory = inventory_for_devices(COLLECTED_AT, Ok(report), &[], channel);
        assert_eq!(inventory.unknown.len(), 180);
        assert!(inventory.report.contains("truncated=true"));
        assert_valid_report(&inventory.report);
        let failure = CollectionFailure::new(Stage::Enumeration,
            "SetupDiEnumDeviceInterfaces 失败（GetLastError=31） private-reason");
        let inventory = inventory_for_devices(COLLECTED_AT, Err(failure), &[], channel);
        assert!(inventory.unknown.is_empty());
        assert!(inventory.report.contains("enumeration=failed"));
        assert!(!inventory.report.contains("private-reason"));
        assert_valid_report(&inventory.report);
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
