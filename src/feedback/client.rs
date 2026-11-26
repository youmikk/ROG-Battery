//! Feedback transport and local recovery. No GitHub token is kept in the app.
//! Only the explicitly supplied draft is sent; device paths and settings are
//! neither read nor added by this module. All public operations are blocking.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    ptr::{null, null_mut},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use winapi::{
    shared::bcrypt::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG},
    um::{
        dpapi::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN},
        winbase::LocalFree,
        wincrypt::DATA_BLOB,
        winhttp::*,
    },
};

#[path = "capture.rs"]
mod capture_attachment;
pub use capture_attachment::{capture, Capture};

const REPOSITORY: &str = "https://github.com/youmikk/X-Battery";
// Existing encrypted history and cached relay receipts use the former URL.
const LEGACY_REPOSITORY: &str = "https://github.com/youmikk/ROG-Battery";
const MAX_IMAGE: usize = 5 * 1024 * 1024;
const MAX_RESPONSE: usize = 512 * 1024;
const MAX_REQUEST: usize = 21 * 1024 * 1024;
const MAX_HISTORY: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const MAX_NUMBER: u64 = 9_007_199_254_740_991;
const MAX_PREFILL_URL: usize = 7500;
const HISTORY_MAGIC: &[u8] = b"RBFH\x01\0\0\0";
static HISTORY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn message(zh: &str, en: &str) -> String {
    if crate::i18n::english() { en.into() } else { zh.into() }
}

fn invalid() -> String {
    message("反馈内容格式不正确，请检查设备信息、说明和图片。",
        "Invalid feedback. Check the device details, description and images.")
}

fn damaged() -> String {
    message("无法读取本地反馈记录。", "The local feedback history could not be read.")
}

fn storage_error() -> String {
    message("反馈记录保存失败，提交结果暂未确认。请检查磁盘空间或权限后重试。",
        "Feedback history could not be saved. The submission result is not confirmed. Check disk space or permissions and retry.")
}

pub(super) fn storage_event(operation: &str, error: &std::io::Error) -> String {
    let code = error.raw_os_error().map(|code| code.to_string()).unwrap_or_else(|| "none".into());
    format!("step={operation} os_error={code}")
}

fn storage_io_error(operation: (&str, &str), error: std::io::Error) -> String {
    let event = storage_event(operation.1, &error);
    #[cfg(not(test))]
    crate::diagnostics::event_once("feedback_storage_failed", &event);
    #[cfg(test)]
    let _ = event;
    format!("{}\n{}: {error}", storage_error(), message(operation.0, operation.1))
}

fn service_error() -> String {
    message("反馈服务返回异常，请稍后重试。",
        "The feedback service returned an unexpected response. Please retry later.")
}

fn network_error() -> String {
    message("无法连接反馈服务，请检查网络后重试。",
        "The feedback service could not be reached. Check your network and retry.")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Device {
    pub name: String,
    pub family: String,
    pub connection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Image {
    pub name: String,
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Draft {
    pub category: String,
    pub device: Device,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<String>,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<Capture>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status { Pending, Completed }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub request_id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub category: String,
    pub device: Device,
    pub description: String,
    pub status: Status,
    pub number: Option<u64>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitOutcome { pub entry: Entry, pub pending: bool }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: u64,
    pub body: String,
    pub author: String,
    pub created_at: String,
    pub url: String,
    pub is_maintainer: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub issue: Issue,
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub comments_truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyReceipt { pub id: u64, pub url: String, pub created_at: String }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fallback {
    pub markdown: String,
    pub url: String,
    pub images_omitted: bool,
    pub capture_omitted: bool,
}

// These types never leave local Rust state or the DPAPI-encrypted file.
// In particular, reply_key is deliberately absent from the public Entry.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredEntry {
    request_id: String,
    reply_key: String,
    app_version: String,
    draft: Draft,
    created_at: u64,
    updated_at: u64,
    status: Status,
    number: Option<u64>,
    url: Option<String>,
    pending_reply: Option<StoredReply>,
    last_reply: Option<FinishedReply>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredReply { request_id: String, body: String }

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinishedReply { request: StoredReply, receipt: ReplyReceipt }

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct History { schema: u8, entries: Vec<StoredEntry> }

impl Default for History {
    fn default() -> Self { Self { schema: 1, entries: Vec::new() } }
}

impl StoredEntry {
    fn public(&self) -> Entry {
        Entry {
            request_id: self.request_id.clone(), created_at: self.created_at,
            updated_at: self.updated_at, category: self.draft.category.clone(),
            device: self.draft.device.clone(), description: self.draft.description.clone(),
            status: self.status, number: self.number,
            url: self.number.map(|number| format!("{REPOSITORY}/issues/{number}")),
        }
    }
}

fn text(value: &str, min: usize, max: usize, multiline: bool) -> bool {
    let count = value.chars().count();
    (min..=max).contains(&count) && (min == 0 || !value.trim().is_empty())
        && !value.chars().any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
}

fn valid_version(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value.bytes().all(|c| c.is_ascii_alphanumeric() || b".+_-".contains(&c))
}

fn uuid(value: &str) -> bool {
    value.len() == 36 && value.bytes().enumerate().all(|(i, b)| {
        if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() }
    }) && value.as_bytes()[14] == b'4' && b"89abAB".contains(&value.as_bytes()[19])
}

fn reply_key(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn valid_device(device: &Device) -> bool {
    text(&device.name, 1, 120, false)
        && matches!(device.family.as_str(), "mouse" | "keyboard" | "headset" | "gamepad" | "other")
        && text(&device.connection, 1, 80, false)
        && device.brand.as_ref().is_none_or(|v| text(v, 1, 80, false))
        && device.model.as_ref().is_none_or(|v| text(v, 1, 120, false))
}

pub fn validate_details(draft: &Draft) -> Result<(), String> {
    if !matches!(draft.category.as_str(), "device" | "battery" | "dpi" | "other")
        || !valid_device(&draft.device) || !text(&draft.description, 5, 2000, true)
        || draft.images.len() > 3 { return Err(invalid()); }
    if let Some(value) = &draft.diagnostics {
        if value.len() > 20 * 1024 || !text(value, 0, 20 * 1024, true) { return Err(invalid()); }
        let lower = value.to_ascii_lowercase();
        if ["serial:", "serial=", "path:", "path=", "alias:", "alias=", "config:",
            "config=", r"\\?\", ":\\", "authorization:", "bearer "]
            .iter().any(|marker| lower.contains(marker)) {
            return Err(message("诊断摘要包含设备标识、路径或凭证，请重新生成摘要。",
                "The diagnostic summary contains identifiers, paths or credentials. Generate a new summary."));
        }
    }
    Ok(())
}

pub fn validate_draft(draft: &Draft) -> Result<(), String> {
    validate_details(draft)?;
    for picture in &draft.images { validate_image(picture)?; }
    if let Some(attachment) = &draft.capture { capture_attachment::validate(attachment)?; }
    Ok(())
}

fn dimensions(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && width <= 16384 && height <= 16384
        && u64::from(width) * u64::from(height) <= 60_000_000
}

const fn crc32_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut index = 0;
    while index < table.len() {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(value & 1));
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

const CRC32_TABLE: [u32; 256] = crc32_table();

fn crc32(bytes: &[u8]) -> u32 {
    let mut checksum = u32::MAX;
    for byte in bytes {
        checksum = (checksum >> 8) ^ CRC32_TABLE[((checksum ^ u32::from(*byte)) & 255) as usize];
    }
    !checksum
}

fn png(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return false; }
    let mut offset = 8;
    let mut header = false;
    let mut data = false;
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let Some(end) = offset.checked_add(12).and_then(|o| o.checked_add(length)) else { return false; };
        if end > bytes.len() { return false; }
        let kind = &bytes[offset + 4..offset + 8];
        if !kind.iter().all(u8::is_ascii_alphabetic)
            || crc32(&bytes[offset + 4..end - 4]) != u32::from_be_bytes(bytes[end - 4..end].try_into().unwrap()) {
            return false;
        }
        if !header {
            if kind != b"IHDR" || length != 13 { return false; }
            let width = u32::from_be_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
            if !dimensions(width, height) { return false; }
            let depth = bytes[offset + 16];
            let color = bytes[offset + 17];
            if !match color { 0 => matches!(depth, 1 | 2 | 4 | 8 | 16), 2 | 4 | 6 => matches!(depth, 8 | 16),
                3 => matches!(depth, 1 | 2 | 4 | 8), _ => false }
                || bytes[offset + 18..offset + 20] != [0, 0] || bytes[offset + 20] > 1 { return false; }
            header = true;
        } else if kind == b"IHDR" { return false; }
        if kind == b"IDAT" && length > 0 { data = true; }
        if kind == b"IEND" { return length == 0 && data && end == bytes.len(); }
        offset = end;
    }
    false
}

fn jpeg(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xff, 0xd8]) || !bytes.ends_with(&[0xff, 0xd9]) { return false; }
    let mut offset = 2;
    let mut frame = false;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff { return false; }
        while offset < bytes.len() && bytes[offset] == 0xff { offset += 1; }
        let Some(&marker) = bytes.get(offset) else { return false; };
        offset += 1;
        if matches!(marker, 0 | 0xd8 | 0xd9 | 0xd0..=0xd7) || offset + 2 > bytes.len() { return false; }
        if marker == 1 { continue; }
        let length = u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize;
        if length < 2 || offset + length > bytes.len() { return false; }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 8 { return false; }
            let height = u16::from_be_bytes(bytes[offset + 3..offset + 5].try_into().unwrap());
            let width = u16::from_be_bytes(bytes[offset + 5..offset + 7].try_into().unwrap());
            if !dimensions(u32::from(width), u32::from(height)) { return false; }
            frame = true;
        }
        if marker == 0xda { return frame && length >= 6 && offset + length < bytes.len() - 2; }
        offset += length;
    }
    false
}

fn webp(bytes: &[u8]) -> bool {
    if bytes.len() < 20 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP"
        || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize != bytes.len() - 8 { return false; }
    let mut offset = 12;
    let mut size = false;
    let mut coded = false;
    while offset + 8 <= bytes.len() {
        let kind = &bytes[offset..offset + 4];
        let length = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let Some(end) = offset.checked_add(8).and_then(|o| o.checked_add(length)) else { return false; };
        if end > bytes.len() { return false; }
        let chunk = &bytes[offset + 8..end];
        if kind == b"VP8X" {
            if length != 10 { return false; }
            let width = 1 + u32::from_le_bytes([chunk[4], chunk[5], chunk[6], 0]);
            let height = 1 + u32::from_le_bytes([chunk[7], chunk[8], chunk[9], 0]);
            if !dimensions(width, height) { return false; }
            size = true;
        } else if kind == b"VP8 " {
            if length < 10 || chunk[3..6] != [0x9d, 1, 0x2a] { return false; }
            let width = u16::from_le_bytes([chunk[6], chunk[7]]) & 0x3fff;
            let height = u16::from_le_bytes([chunk[8], chunk[9]]) & 0x3fff;
            if !dimensions(u32::from(width), u32::from(height)) { return false; }
            size = true; coded = true;
        } else if kind == b"VP8L" {
            if length < 5 || chunk[0] != 0x2f { return false; }
            let packed = u32::from_le_bytes(chunk[1..5].try_into().unwrap());
            if packed >> 29 != 0 || !dimensions((packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1) {
                return false;
            }
            size = true; coded = true;
        } else if kind == b"ANMF" {
            if !size || length < 24 { return false; }
            coded = true;
        }
        offset = end + length % 2;
    }
    offset == bytes.len() && size && coded
}

fn valid_image_bytes(mime: &str, bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.len() <= MAX_IMAGE && match mime {
        "image/png" => png(bytes), "image/jpeg" => jpeg(bytes), "image/webp" => webp(bytes), _ => false,
    }
}

fn validate_image(picture: &Image) -> Result<(), String> {
    if !text(&picture.name, 1, 160, false) || picture.name.contains(['/', '\\', ':'])
        || matches!(picture.name.as_str(), "." | "..")
        || picture.data_base64.len() > (MAX_IMAGE + 2) / 3 * 4 { return Err(invalid()); }
    let bytes = STANDARD.decode(&picture.data_base64).map_err(|_| invalid())?;
    if STANDARD.encode(&bytes) != picture.data_base64 || !valid_image_bytes(&picture.mime_type, &bytes) {
        return Err(message("图片必须是有效的 PNG、JPEG 或 WebP，每张不超过 5 MiB，最多 3 张。",
            "Use valid PNG, JPEG or WebP images, up to 5 MiB each and no more than three images."));
    }
    Ok(())
}

pub fn image(name: &str, mime_type: &str, bytes: &[u8]) -> Result<Image, String> {
    if bytes.len() > MAX_IMAGE { return Err(invalid()); }
    let result = Image { name: name.into(), mime_type: mime_type.into(), data_base64: STANDARD.encode(bytes) };
    validate_image(&result)?;
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Endpoint { host: String, port: u16, base: String }

fn parse_endpoint(value: &str) -> Option<Endpoint> {
    if value.len() > 2048 || !value.is_ascii() || value.bytes().any(|b| b <= 32 || b == 127)
        || value.contains(['@', '?', '#', '\\', '%']) { return None; }
    let rest = value.strip_prefix("https://")?;
    let (authority, base) = rest.split_once('/').map_or((rest, ""), |(host, path)| (host, path));
    let (host, port) = if let Some((host, port)) = authority.split_once(':') {
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) { return None; }
        (host, port.parse::<u16>().ok()?)
    } else { (authority, 443) };
    if host.is_empty() || host.len() > 253 || port == 0 || host.ends_with('.')
        || !host.split('.').all(|label| !label.is_empty() && label.len() <= 63
            && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) { return None; }
    let base = base.trim_end_matches('/');
    if !base.is_empty() && !base.split('/').all(|part| !part.is_empty() && part != "." && part != ".."
        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))) { return None; }
    Some(Endpoint { host: host.to_ascii_lowercase(), port, base: if base.is_empty() { String::new() }
        else { format!("/{base}") } })
}

pub fn endpoint() -> Option<&'static str> {
    option_env!("R_BATTERY_FEEDBACK_URL").filter(|value| parse_endpoint(value).is_some())
}

pub fn configured() -> bool { endpoint().is_some() }

fn required_endpoint() -> Result<Endpoint, String> {
    option_env!("R_BATTERY_FEEDBACK_URL").and_then(parse_endpoint).ok_or_else(|| message(
        "当前版本暂未开通应用内直投，请先复制报告前往 GitHub 提交，如有截图或抓包文件记得一并上传。",
        "In-app submission is not enabled in this build. Copy the report to submit on GitHub and attach screenshots or capture files manually."))
}

fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }

struct Internet(HINTERNET);
impl Drop for Internet { fn drop(&mut self) { unsafe { WinHttpCloseHandle(self.0); } } }

fn internet(handle: HINTERNET) -> Result<Internet, String> {
    if handle.is_null() { Err(network_error()) } else { Ok(Internet(handle)) }
}

fn option(handle: HINTERNET, key: u32, mut value: u32) -> Result<(), String> {
    if unsafe { WinHttpSetOption(handle, key, (&mut value as *mut u32).cast(), 4) } == 0 {
        Err(network_error())
    } else { Ok(()) }
}

fn http(endpoint: &Endpoint, method: &str, route: &str, bearer: Option<&str>, body: &[u8])
    -> Result<(u32, Vec<u8>), String> {
    if !matches!(method, "GET" | "POST") || !route.starts_with("/feedback")
        || !route.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'/')
        || body.len() > MAX_REQUEST || bearer.is_some_and(|key| !reply_key(key)) { return Err(invalid()); }
    let agent = wide("X-Battery feedback/1");
    let session = internet(unsafe { WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        null(), null(), 0) })?;
    if unsafe { WinHttpSetTimeouts(session.0, 5000, 5000, 15000, 15000) } == 0 { return Err(network_error()); }
    // TLS 1.2, normal certificate verification, no ambient Windows credentials.
    option(session.0, WINHTTP_OPTION_SECURE_PROTOCOLS, 0x00000800)?;
    let host = wide(&endpoint.host);
    let connection = internet(unsafe { WinHttpConnect(session.0, host.as_ptr(), endpoint.port, 0) })?;
    let verb = wide(method);
    let path = wide(&format!("{}{route}", endpoint.base));
    let request = internet(unsafe { WinHttpOpenRequest(connection.0, verb.as_ptr(), path.as_ptr(),
        null(), null(), null_mut(), WINHTTP_FLAG_SECURE | WINHTTP_FLAG_REFRESH) })?;
    option(request.0, WINHTTP_OPTION_REDIRECT_POLICY, 0)?;
    option(request.0, WINHTTP_OPTION_AUTOLOGON_POLICY, 2)?;
    // WINHTTP_DISABLE_COOKIES | REDIRECTS | AUTHENTICATION.
    option(request.0, WINHTTP_OPTION_DISABLE_FEATURE, 1 | 2 | 4)?;
    option(request.0, WINHTTP_OPTION_MAX_RESPONSE_HEADER_SIZE, 16 * 1024)?;
    option(request.0, WINHTTP_OPTION_CONNECT_RETRIES, 1)?;
    let mut headers = String::from("Accept: application/json\r\nContent-Type: application/json; charset=utf-8\r\n");
    if let Some(key) = bearer { headers.push_str("Authorization: Bearer "); headers.push_str(key); headers.push_str("\r\n"); }
    let headers = wide(&headers);
    let started = Instant::now();
    if unsafe { WinHttpSendRequest(request.0, headers.as_ptr(), (headers.len() - 1) as u32,
        if body.is_empty() { null_mut() } else { body.as_ptr() as *mut _ }, body.len() as u32,
        body.len() as u32, 0) } == 0 || unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(network_error());
    }
    let mut status = 0u32;
    let mut length = 4u32;
    if unsafe { WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        null(), (&mut status as *mut u32).cast(), &mut length, null_mut()) } == 0 { return Err(network_error()); }
    if status == 429 {
        let header = wide("Retry-After");
        let mut buffer = [0u16; 32];
        let mut length = (buffer.len() * 2) as u32;
        let value = if unsafe { WinHttpQueryHeaders(request.0, WINHTTP_QUERY_CUSTOM, header.as_ptr(),
            buffer.as_mut_ptr().cast(), &mut length, null_mut()) } != 0 {
            Some(String::from_utf16_lossy(&buffer[..buffer.iter().position(|value| *value == 0)
                .unwrap_or(buffer.len())]))
        } else { None };
        return Err(rate_limit_error(value.as_deref()));
    }
    // Only this relay status can carry an explicit terminal rejection. Other
    // errors remain uncertain and must not release a pending request.
    if !matches!(status, 200 | 201 | 202 | 502) { return Err(http_error(status)); }
    let mut content = [0u16; 128];
    let mut content_length = (content.len() * 2) as u32;
    if unsafe { WinHttpQueryHeaders(request.0, WINHTTP_QUERY_CONTENT_TYPE, null(), content.as_mut_ptr().cast(),
        &mut content_length, null_mut()) } == 0 { return Err(service_error()); }
    let content = String::from_utf16_lossy(&content[..content.iter().position(|c| *c == 0).unwrap_or(content.len())]);
    if !content.split(';').next().unwrap_or_default().trim().eq_ignore_ascii_case("application/json") {
        return Err(service_error());
    }
    let mut output = Vec::new();
    loop {
        let Some(left) = Duration::from_secs(45).checked_sub(started.elapsed()) else { return Err(network_error()); };
        option(request.0, WINHTTP_OPTION_RECEIVE_TIMEOUT, left.as_millis().min(15000).max(1) as u32)?;
        let mut buffer = [0u8; 8192];
        let mut read = 0;
        if unsafe { WinHttpReadData(request.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut read) } == 0 {
            return Err(network_error());
        }
        if read == 0 { break; }
        let limit = if status == 502 { 4096 } else { MAX_RESPONSE };
        if read as usize > buffer.len() || output.len() + read as usize > limit { return Err(service_error()); }
        output.extend_from_slice(&buffer[..read as usize]);
    }
    if status == 502 && !rejected(status, &output) { return Err(http_error(status)); }
    Ok((status, output))
}

#[derive(Deserialize)]
struct ServiceFailure { error: FailureCode }
#[derive(Deserialize)]
struct FailureCode { code: String, retryable: bool }

fn rejected(status: u32, bytes: &[u8]) -> bool {
    status == 502 && bytes.len() <= 4096 && serde_json::from_slice::<ServiceFailure>(bytes)
        .is_ok_and(|failure| failure.error.code == "github_rejected" && !failure.error.retryable)
}

fn http_error(status: u32) -> String {
    match status {
        401 | 403 => message("当前设备暂未匹配到这条反馈的跟进权限，如需查看或补充，请直接前往 GitHub 页面。",
            "This computer does not have permission to update that feedback. View or update the issue on GitHub."),
        404 => message("未找到这条反馈，请稍后重试或前往 GitHub 查看。",
            "This feedback could not be found. Retry later or view it on GitHub."),
        409 => message("反馈请求暂时无法完成，请稍后重试。",
            "The feedback request cannot be completed right now. Please retry later."),
        413 => message("反馈附件超出服务限制，请通过 GitHub 提交。", "The attachments exceed the service limit. Submit them on GitHub."),
        429 => message("提交过于频繁，请稍后重试。", "Too many requests. Please retry later."),
        400 | 422 => message("反馈内容未被接受，请检查内容后重试。",
            "The feedback was not accepted. Check the content and retry."),
        _ => network_error(),
    }
}

fn rate_limit_error(retry_after: Option<&str>) -> String {
    let Some(seconds) = retry_after.and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|seconds| (1..=86400).contains(seconds)) else { return http_error(429); };
    let wait = if seconds >= 3600 {
        format!("{} {}", seconds.div_ceil(3600), message("小时", "hour(s)"))
    } else if seconds >= 60 {
        format!("{} {}", seconds.div_ceil(60), message("分钟", "minute(s)"))
    } else {
        format!("{} {}", seconds, message("秒", "second(s)"))
    };
    message(&format!("提交过于频繁，请在约 {wait}后重试。"),
        &format!("Too many requests. Retry in about {wait}."))
}

fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0; N];
    if unsafe { BCryptGenRandom(null_mut(), bytes.as_mut_ptr(), N as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) } < 0 {
        return Err(message("无法创建反馈请求，请重试。", "The feedback request could not be created. Please retry."));
    }
    Ok(bytes)
}

fn new_uuid() -> Result<String, String> {
    let mut bytes = random::<16>()?;
    bytes[6] = (bytes[6] & 15) | 0x40;
    bytes[8] = (bytes[8] & 63) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(u128::from(u64::MAX)) as u64 }

struct ProtectedBlob(DATA_BLOB);
impl Drop for ProtectedBlob {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            unsafe { LocalFree(self.0.pbData.cast()); }
        }
    }
}

fn crypt(bytes: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    if bytes.is_empty() || bytes.len() > MAX_HISTORY { return Err(damaged()); }
    let mut input = DATA_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 };
    let mut output = ProtectedBlob(DATA_BLOB { cbData: 0, pbData: null_mut() });
    let ok = unsafe {
        if protect {
            CryptProtectData(&mut input, null(), null_mut(), null_mut(), null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN, &mut output.0)
        } else {
            CryptUnprotectData(&mut input, null_mut(), null_mut(), null_mut(), null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN, &mut output.0)
        }
    };
    if ok == 0 && protect {
        return Err(storage_io_error(("加密本地记录", "Encrypt local history"), std::io::Error::last_os_error()));
    }
    if ok == 0 || output.0.pbData.is_null() || output.0.cbData as usize > MAX_HISTORY {
        return Err(if protect { storage_error() } else { damaged() });
    }
    Ok(unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec())
}

fn history_path() -> std::path::PathBuf { crate::paths::config_dir().join("feedback-history.dat") }

fn read_history(path: &Path) -> Result<History, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(History::default()),
        Err(_) => return Err(damaged()),
    };
    if file.metadata().map_err(|_| damaged())?.len() > MAX_HISTORY as u64 { return Err(damaged()); }
    let mut encrypted = Vec::new();
    (&mut file).take(MAX_HISTORY as u64 + 1).read_to_end(&mut encrypted).map_err(|_| damaged())?;
    if encrypted.len() > MAX_HISTORY || !encrypted.starts_with(HISTORY_MAGIC) { return Err(damaged()); }
    let plain = crypt(&encrypted[HISTORY_MAGIC.len()..], false)?;
    let history: History = serde_json::from_slice(&plain).map_err(|_| damaged())?;
    validate_history(&history)?;
    Ok(history)
}

fn save_history(path: &Path, history: &History) -> Result<(), String> {
    validate_history(history)?;
    let plain = serde_json::to_vec(history).map_err(|_| storage_error())?;
    if plain.len() > MAX_HISTORY - 65536 { return Err(storage_error()); }
    let encrypted = crypt(&plain, true)?;
    let parent = path.parent().ok_or_else(storage_error)?;
    fs::create_dir_all(parent).map_err(|error| storage_io_error(("创建保存目录", "Create history directory"), error))?;
    let temp = parent.join(format!("feedback-history-{}.tmp", new_uuid()?));
    let result = (|| {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&temp)
            .map_err(|error| storage_io_error(("创建临时记录", "Create temporary history"), error))?;
        file.write_all(HISTORY_MAGIC).and_then(|_| file.write_all(&encrypted)).and_then(|_| file.sync_all())
            .map_err(|error| storage_io_error(("写入本地记录", "Write local history"), error))?;
        drop(file);
        crate::paths::replace_file(&temp, path)
            .map_err(|error| storage_io_error(("安全替换本地记录", "Replace local history"), error))?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temp); }
    result
}

fn lock_history(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let parent = path.parent().ok_or_else(storage_error)?;
    fs::create_dir_all(parent).map_err(|error| storage_io_error(("创建保存目录", "Create history directory"), error))?;
    let lock = parent.join("feedback-history.lock");
    let started = Instant::now();
    loop {
        match OpenOptions::new().read(true).write(true).create(true).share_mode(0).open(&lock) {
            Ok(file) => return Ok(file),
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33))
                && started.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(storage_io_error(("锁定本地记录", "Lock local history"), error)),
        }
    }
}

fn with_history<T>(write: bool, action: impl FnOnce(&mut History) -> Result<T, String>) -> Result<T, String> {
    let _guard = HISTORY_LOCK.get_or_init(Default::default).lock().map_err(|_| damaged())?;
    let path = history_path();
    let _history_lock = if write { Some(lock_history(&path)?) } else { None };
    let mut history = read_history(&path)?;
    let result = action(&mut history)?;
    if write { save_history(&path, &history)?; }
    Ok(result)
}

fn valid_number(number: u64) -> bool { number > 0 && number <= MAX_NUMBER }
fn issue_url(number: u64, url: &str) -> bool {
    valid_number(number) && [REPOSITORY, LEGACY_REPOSITORY].iter()
        .any(|repository| url == format!("{repository}/issues/{number}"))
}
fn comment_url(number: u64, id: u64, url: &str) -> bool {
    valid_number(number) && valid_number(id) && [REPOSITORY, LEGACY_REPOSITORY].iter()
        .any(|repository| url == format!("{repository}/issues/{number}#issuecomment-{id}"))
}

fn timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20 || bytes.len() > 35 || !value.is_ascii()
        || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' {
        return false;
    }
    let parse = |a, b| value.get(a..b).filter(|s| s.bytes().all(|c| c.is_ascii_digit()))?.parse::<u32>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) =
        (parse(0, 4), parse(5, 7), parse(8, 10), parse(11, 13), parse(14, 16), parse(17, 19)) else { return false; };
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 { return false; }
    let days = match month { 4 | 6 | 9 | 11 => 30, 2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28, _ => 31 };
    if day == 0 || day > days { return false; }
    let suffix = &value[19..];
    suffix == "Z" || suffix.strip_prefix('.').and_then(|s| s.strip_suffix('Z'))
        .is_some_and(|s| !s.is_empty() && s.len() <= 9 && s.bytes().all(|c| c.is_ascii_digit()))
}

fn valid_receipt(number: u64, value: &ReplyReceipt) -> bool {
    comment_url(number, value.id, &value.url) && timestamp(&value.created_at)
}

fn validate_history(history: &History) -> Result<(), String> {
    if history.schema != 1 || history.entries.len() > MAX_ENTRIES { return Err(damaged()); }
    let mut ids = BTreeSet::new();
    let mut numbers = BTreeSet::new();
    for entry in &history.entries {
        if !uuid(&entry.request_id) || !reply_key(&entry.reply_key) || !valid_version(&entry.app_version)
            || !ids.insert(entry.request_id.as_str()) || entry.created_at == 0
            || entry.updated_at < entry.created_at || entry.updated_at > 253_402_300_799_999
            || validate_draft(&entry.draft).is_err() { return Err(damaged()); }
        match entry.status {
            Status::Pending if entry.number.is_none() && entry.url.is_none()
                && entry.pending_reply.is_none() && entry.last_reply.is_none() => (),
            Status::Completed if entry.number.zip(entry.url.as_deref())
                .is_some_and(|(n, url)| issue_url(n, url) && numbers.insert(n)) => (),
            _ => return Err(damaged()),
        }
        let valid_reply = |r: &StoredReply| uuid(&r.request_id) && text(&r.body, 1, 2000, true);
        if entry.pending_reply.as_ref().is_some_and(|r| !valid_reply(r))
            || entry.last_reply.as_ref().is_some_and(|r| !valid_reply(&r.request)
                || !valid_receipt(entry.number.unwrap_or_default(), &r.receipt)) { return Err(damaged()); }
    }
    Ok(())
}

pub fn load_history() -> Result<Vec<Entry>, String> {
    with_history(false, |history| Ok(history.entries.iter().rev().map(StoredEntry::public).collect()))
}

pub fn load_draft(request_id: &str) -> Result<Draft, String> {
    if !uuid(request_id) { return Err(invalid()); }
    with_history(false, |history| history.entries.iter().find(|entry| entry.request_id == request_id)
        .map(|entry| entry.draft.clone()).ok_or_else(|| message("未找到本地反馈记录。", "The local feedback record was not found.")))
}

/// Recover the persisted request ID after an unsuccessful submit. The caller
/// must pass its original draft snapshot, including the diagnostic summary.
pub fn find_pending(draft: &Draft) -> Result<Option<Entry>, String> {
    validate_draft(draft)?;
    with_history(false, |history| Ok(history.entries.iter().find(|entry|
        entry.status == Status::Pending && &entry.draft == draft).map(StoredEntry::public)))
}

pub fn load_pending_reply(number: u64) -> Result<Option<String>, String> {
    Ok(stored_number(number)?.pending_reply.map(|reply| reply.body))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest<'a> {
    schema: u8, request_id: &'a str, reply_key: &'a str, app_version: &'a str,
    #[serde(flatten)] draft: &'a Draft,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateReceipt { number: u64, url: String, reply_access: bool }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pending { status: String, retry_after: u32 }

fn pending(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Pending>(bytes).is_ok_and(|value| value.status == "pending" && (1..=3600).contains(&value.retry_after))
}

pub fn submit(draft: Draft) -> Result<SubmitOutcome, String> {
    let endpoint = required_endpoint()?;
    validate_draft(&draft)?;
    let version = crate::gui::APP_VERSION;
    if !valid_version(version) { return Err(invalid()); }
    let stored = with_history(true, |history| prepare_entry(history, draft, version))?;
    send_entry(&endpoint, &stored)
}

fn prepare_entry(history: &mut History, draft: Draft, version: &str) -> Result<StoredEntry, String> {
    // A second click or a resumed form reuses the persisted idempotency key.
    if let Some(entry) = history.entries.iter().find(|e| e.status == Status::Pending && e.draft == draft) {
        return Ok(entry.clone());
    }
    if history.entries.len() >= MAX_ENTRIES { return Err(message("本地反馈记录已达到上限，请通过 GitHub 提交。",
        "The local feedback history is full. Please submit on GitHub.")); }
    let time = now();
    let entry = StoredEntry { request_id: new_uuid()?, reply_key: random::<32>()?.iter().map(|b| format!("{b:02x}")).collect(),
        app_version: version.into(), draft, created_at: time, updated_at: time, status: Status::Pending,
        number: None, url: None, pending_reply: None, last_reply: None };
    history.entries.push(entry.clone());
    Ok(entry)
}

pub fn retry(request_id: &str) -> Result<SubmitOutcome, String> {
    let endpoint = required_endpoint()?;
    if !uuid(request_id) { return Err(invalid()); }
    let stored = with_history(false, |history| history.entries.iter().find(|e| e.request_id == request_id)
        .cloned().ok_or_else(|| message("未找到待提交的反馈。", "The pending feedback was not found.")))?;
    if stored.status == Status::Completed { return Ok(SubmitOutcome { entry: stored.public(), pending: false }); }
    send_entry(&endpoint, &stored)
}

fn send_entry(endpoint: &Endpoint, stored: &StoredEntry) -> Result<SubmitOutcome, String> {
    let body = serde_json::to_vec(&CreateRequest { schema: 1, request_id: &stored.request_id,
        reply_key: &stored.reply_key, app_version: &stored.app_version, draft: &stored.draft }).map_err(|_| invalid())?;
    let (status, bytes) = http(endpoint, "POST", "/feedback", None, &body)?;
    if rejected(status, &bytes) {
        return Err(message("GitHub 未接受这份反馈，请检查内容后重试或前往 GitHub 提交。",
            "GitHub did not accept this feedback. Check the content and retry, or submit on GitHub."));
    }
    if status == 202 {
        if !pending(&bytes) { return Err(service_error()); }
        return Ok(SubmitOutcome { entry: stored.public(), pending: true });
    }
    let receipt: CreateReceipt = serde_json::from_slice(&bytes).map_err(|_| service_error())?;
    if !receipt.reply_access || !issue_url(receipt.number, &receipt.url) { return Err(service_error()); }
    let entry = with_history(true, |history| complete_entry(history, stored, receipt))?;
    Ok(SubmitOutcome { entry, pending: false })
}

fn complete_entry(history: &mut History, stored: &StoredEntry, receipt: CreateReceipt) -> Result<Entry, String> {
    if !receipt.reply_access || !issue_url(receipt.number, &receipt.url) { return Err(service_error()); }
    let item = history.entries.iter_mut().find(|e| e.request_id == stored.request_id).ok_or_else(damaged)?;
    if item.reply_key != stored.reply_key || (item.status == Status::Completed
        && (item.number != Some(receipt.number)
            || !item.url.as_deref().is_some_and(|url| issue_url(receipt.number, url)))) { return Err(service_error()); }
    item.status = Status::Completed;
    item.number = Some(receipt.number);
    item.url = Some(format!("{REPOSITORY}/issues/{}", receipt.number));
    item.updated_at = now().max(item.updated_at);
    // Images remain available for pending retries; completed receipts no
    // longer need large local attachment copies.
    item.draft.images.clear();
    item.draft.capture = None;
    Ok(item.public())
}

fn stored_number(number: u64) -> Result<StoredEntry, String> {
    if !valid_number(number) { return Err(invalid()); }
    with_history(false, |history| history.entries.iter().find(|e| e.status == Status::Completed && e.number == Some(number))
        .cloned().ok_or_else(|| message("当前设备没有这条反馈的跟进验证信息，请前往 GitHub 页面查看。",
            "This computer has no verification key for that feedback. View it on GitHub.")))
}

fn valid_thread(number: u64, value: &Thread) -> bool {
    let issue = &value.issue;
    if issue.number != number || !issue_url(number, &issue.url) || !text(&issue.title, 1, 256, false)
        || !matches!(issue.state.as_str(), "open" | "closed") || !text(&issue.body, 0, 65536, true)
        || !timestamp(&issue.created_at) || !timestamp(&issue.updated_at) || value.comments.len() > 300 { return false; }
    let mut ids = BTreeSet::new();
    value.comments.iter().all(|comment| comment.is_maintainer && ids.insert(comment.id)
        && comment_url(number, comment.id, &comment.url) && text(&comment.body, 0, 65536, true)
        && text(&comment.author, 1, 100, false) && timestamp(&comment.created_at))
}

pub fn refresh(number: u64) -> Result<Thread, String> {
    let endpoint = required_endpoint()?;
    let stored = stored_number(number)?;
    let (status, bytes) = http(&endpoint, "GET", &format!("/feedback/{number}"), Some(&stored.reply_key), &[])?;
    if status != 200 { return Err(service_error()); }
    let thread: Thread = serde_json::from_slice(&bytes).map_err(|_| service_error())?;
    if !valid_thread(number, &thread) { return Err(service_error()); }
    Ok(thread)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplyRequest<'a> { schema: u8, request_id: &'a str, body: &'a str }

pub fn reply(number: u64, body: &str) -> Result<ReplyReceipt, String> {
    let endpoint = required_endpoint()?;
    if !valid_number(number) || !text(body, 1, 2000, true) { return Err(invalid()); }
    let stored = with_history(true, |history| {
        let entry = history.entries.iter_mut().find(|e| e.status == Status::Completed && e.number == Some(number))
            .ok_or_else(|| message("当前设备没有这条反馈的跟进验证信息。", "This computer has no verification key for that feedback."))?;
        prepare_reply(entry, body)?;
        Ok(entry.clone())
    })?;
    let pending_reply = stored.pending_reply.as_ref().ok_or_else(damaged)?;
    let payload = serde_json::to_vec(&ReplyRequest { schema: 1, request_id: &pending_reply.request_id,
        body: &pending_reply.body }).map_err(|_| invalid())?;
    let (status, bytes) = http(&endpoint, "POST", &format!("/feedback/{number}/replies"), Some(&stored.reply_key), &payload)?;
    if rejected(status, &bytes) {
        with_history(true, |history| {
            let entry = history.entries.iter_mut().find(|e| e.request_id == stored.request_id).ok_or_else(damaged)?;
            clear_rejected_reply(entry, &stored, pending_reply, status, &bytes);
            Ok(())
        })?;
        return Err(message("GitHub 未接受这条回复，请检查内容后重试或前往 GitHub 回复。",
            "GitHub did not accept this reply. Check the content and retry, or reply on GitHub."));
    }
    if status == 202 {
        if !pending(&bytes) { return Err(service_error()); }
        return Err(message("这条补充回复暂未确认送达，内容已帮你留存，请稍后重试发送。",
            "This reply has not been confirmed yet. Your text is saved locally; retry shortly."));
    }
    let receipt: ReplyReceipt = serde_json::from_slice(&bytes).map_err(|_| service_error())?;
    if !valid_receipt(number, &receipt) { return Err(service_error()); }
    with_history(true, |history| {
        let entry = history.entries.iter_mut().find(|e| e.request_id == stored.request_id).ok_or_else(damaged)?;
        complete_reply(entry, &stored, pending_reply, &receipt)
    })?;
    Ok(receipt)
}

fn prepare_reply(entry: &mut StoredEntry, body: &str) -> Result<(), String> {
    if let Some(pending) = &entry.pending_reply {
        if pending.body != body { return Err(message("上一条补充回复还没确认送达，请先重试发送原内容。",
            "Your previous reply is still awaiting confirmation. Retry sending it first.")); }
    } else {
        // Only an unfinished operation is a retry. After confirmation, a new
        // user action creates another reply even when its text is identical.
        entry.pending_reply = Some(StoredReply { request_id: new_uuid()?, body: body.into() });
        entry.updated_at = now().max(entry.updated_at);
    }
    Ok(())
}

fn clear_rejected_reply(entry: &mut StoredEntry, stored: &StoredEntry, pending: &StoredReply,
    status: u32, bytes: &[u8]) -> bool {
    if !rejected(status, bytes) || entry.reply_key != stored.reply_key || entry.number != stored.number
        || entry.status != Status::Completed || !entry.pending_reply.as_ref().is_some_and(|r|
            r.request_id == pending.request_id && r.body == pending.body) { return false; }
    entry.pending_reply = None;
    entry.updated_at = now().max(entry.updated_at);
    true
}

fn complete_reply(entry: &mut StoredEntry, stored: &StoredEntry, pending: &StoredReply, receipt: &ReplyReceipt)
    -> Result<(), String> {
    if entry.reply_key != stored.reply_key || !valid_receipt(entry.number.unwrap_or_default(), receipt) { return Err(damaged()); }
    if entry.pending_reply.as_ref().is_some_and(|r| r.request_id == pending.request_id && r.body == pending.body) {
        let mut receipt = receipt.clone();
        receipt.url = format!("{REPOSITORY}/issues/{}#issuecomment-{}", entry.number.unwrap(), receipt.id);
        entry.last_reply = Some(FinishedReply { request: pending.clone(), receipt });
        entry.pending_reply = None;
        entry.updated_at = now().max(entry.updated_at);
    } else if !entry.last_reply.as_ref().is_some_and(|r| r.request.request_id == pending.request_id
        && r.receipt.id == receipt.id
        && comment_url(entry.number.unwrap_or_default(), r.receipt.id, &r.receipt.url)) { return Err(service_error()); }
    Ok(())
}

fn encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) { encoded.push(byte as char); }
        else { encoded.push_str(&format!("%{byte:02X}")); }
    }
    encoded
}

fn encoded_prefix(value: &str, budget: usize) -> String {
    let mut encoded = String::new();
    for character in value.chars() {
        let mut utf8 = [0; 4];
        let part = encode(character.encode_utf8(&mut utf8));
        if encoded.len() + part.len() > budget { break; }
        encoded.push_str(&part);
    }
    encoded
}

fn markdown_label(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('\\', "\\\\").replace(['[', ']'], "").replace('*', "\\*").replace('_', "\\_").replace('`', "\\`")
}

pub fn fallback(draft: &Draft) -> Result<Fallback, String> {
    validate_draft(draft)?;
    if !valid_version(crate::gui::APP_VERSION) { return Err(invalid()); }
    let title = format!("[{}] {}", match draft.category.as_str() {
        "device" => message("设备适配", "Device support"), "battery" => message("电量", "Battery"),
        "dpi" => "DPI".into(), _ => message("反馈", "Feedback"),
    }, draft.device.name);
    let header = format!("X-Battery {}\n\n- Device: {}\n- Type: {}\n- Connection: {}\n", crate::gui::APP_VERSION,
        markdown_label(&draft.device.name), draft.device.family, markdown_label(&draft.device.connection));
    let mut markdown = header.clone();
    if let Some(brand) = &draft.device.brand { markdown.push_str(&format!("- Brand: {}\n", markdown_label(brand))); }
    if let Some(model) = &draft.device.model { markdown.push_str(&format!("- Model: {}\n", markdown_label(model))); }
    markdown.push_str(&format!("\n{}\n", draft.description));
    if let Some(diagnostics) = &draft.diagnostics {
        let longest = diagnostics.split(|c| c != '`').map(str::len).max().unwrap_or_default();
        let fence = "`".repeat((longest + 1).max(3));
        markdown.push_str(&format!("\n<details><summary>Diagnostics</summary>\n\n{fence}text\n{diagnostics}\n{fence}\n\n</details>\n"));
    }
    let note = if draft.capture.is_some() {
        message("附件未通过此链接上传，请在 GitHub 页面手动添加图片和原始采集文件。",
            "Attachments are not uploaded through this link. Add images and raw capture files manually on GitHub.")
    } else {
        message("图片未通过此链接上传，请在 GitHub 页面手动添加。",
            "Images are not uploaded through this link. Add them manually on GitHub.")
    };
    if !draft.images.is_empty() || draft.capture.is_some() { markdown.push_str(&format!("\n{note}\n")); }
    let prefix = format!("{REPOSITORY}/issues/new?title={}&body=", encode(&title));
    let full_url = format!("{prefix}{}", encode(&markdown));
    let url = if full_url.len() <= MAX_PREFILL_URL { full_url } else {
        let instructions = message("请在提交前粘贴完整报告和诊断摘要。", "Paste the complete report and diagnostic summary before submitting.");
        let prefix = format!("{REPOSITORY}/issues/new?title={}&body=", encoded_prefix(&title, 1024));
        let footer = encode(&format!("\n\n{instructions}\n{note}"));
        let budget = MAX_PREFILL_URL.checked_sub(prefix.len() + footer.len()).ok_or_else(invalid)?;
        // Reserve room for the explanation and attachment notice first. Every
        // cut is at a Unicode boundary, including in percent-encoded UTF-8.
        let summary = encoded_prefix(&format!("{header}\n"), budget / 2);
        let description = encoded_prefix(&draft.description, budget - summary.len());
        format!("{prefix}{summary}{description}{footer}")
    };
    Ok(Fallback { markdown, url, images_omitted: !draft.images.is_empty(), capture_omitted: draft.capture.is_some() })
}

#[cfg(test)]
mod tests {
    use super::*;
    const REQUEST: &str = "12345678-1234-4234-8234-123456789abc";

    #[test]
    fn png_crc_matches_the_standard_and_previous_bitwise_algorithm() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(&[]), 0);
        let bytes: Vec<u8> = (0..100_000).map(|index| (index * 37) as u8).collect();
        let mut expected = u32::MAX;
        for byte in &bytes {
            expected ^= u32::from(*byte);
            for _bit in 0..8 {
                expected = (expected >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(expected & 1));
            }
        }
        assert_eq!(crc32(&bytes), !expected);
    }

    #[test]
    fn form_validation_leaves_attachment_scanning_to_the_worker() {
        let mut report = draft();
        report.images.push(Image {
            name: "screenshot.png".into(), mime_type: "image/png".into(), data_base64: "invalid-base64".into(),
        });
        assert!(validate_details(&report).is_ok());
        assert!(validate_draft(&report).is_err());
        report.images.clear();
        report.capture = Some(Capture { name: "raw.jsonl".into(), mime_type: "application/x-ndjson".into(),
            data_base64: "invalid-base64".into() });
        assert!(validate_details(&report).is_ok());
        assert!(validate_draft(&report).is_err());
        report.description = "no".into();
        assert!(validate_details(&report).is_err());
    }

    #[test]
    fn completed_feedback_does_not_block_a_second_identical_submission() {
        let mut history = History::default();
        let original = prepare_entry(&mut history, draft(), "1.4.9").unwrap();
        complete_entry(&mut history, &original, CreateReceipt {
            number: 123, url: format!("{REPOSITORY}/issues/123"), reply_access: true,
        }).unwrap();
        let second = prepare_entry(&mut history, draft(), "1.4.9").unwrap();
        assert_ne!(original.request_id, second.request_id);
        assert_ne!(original.reply_key, second.reply_key);
        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries[0].status, Status::Completed);
        assert_eq!(history.entries[1].status, Status::Pending);
        assert!(validate_history(&history).is_ok());
    }

    #[test]
    fn a_new_report_preserves_the_previous_uncertain_submission() {
        let mut history = History::default();
        let original = prepare_entry(&mut history, draft(), "1.4.9").unwrap();
        let mut next = draft();
        next.description = "A different battery problem".into();
        let second = prepare_entry(&mut history, next, "1.4.9").unwrap();
        assert_ne!(original.request_id, second.request_id);
        assert_eq!(history.entries[0].request_id, original.request_id);
        assert_eq!(history.entries[0].reply_key, original.reply_key);
        assert_eq!(history.entries[0].status, Status::Pending);
    }

    #[test]
    fn rate_limit_messages_report_wait_times_and_reject_invalid_headers() {
        assert!(rate_limit_error(Some("37")).contains("37"));
        assert!(rate_limit_error(Some("61")).contains('2'));
        assert!(rate_limit_error(Some("3600")).contains('1'));
        assert!(rate_limit_error(Some("86400")).contains("24"));
        for value in [None, Some("0"), Some("-1"), Some("86401"), Some("999999999999999999"),
            Some("tomorrow")] {
            assert_eq!(rate_limit_error(value), http_error(429));
        }
    }

    fn draft() -> Draft {
        Draft { category: "battery".into(), device: Device { name: "Example mouse".into(), family: "mouse".into(),
            connection: "2.4 GHz".into(), brand: Some("Brand".into()), model: None },
            description: "Battery reading is unavailable.".into(), diagnostics: None, images: vec![], capture: None }
    }

    fn raw_capture(hex: &str) -> Vec<u8> {
        let header = serde_json::json!({"schema":1, "app":"1.4.10", "started_unix_ms":1795651200000u64,
            "vendor_id":4660, "product_id":22136, "usage_page":65280, "usage":1,
            "input_bytes":64, "output_bytes":64, "feature_bytes":0,
            "scope":"one interface; all receiver slots; existing application I/O only; timestamps are completion times",
            "privacy":"Raw data may contain device identifiers or input information. Included captures are shared publicly with feedback."});
        let transfer = serde_json::json!({"kind":"transfer", "elapsed_ms":1, "api":"input", "success":true, "hex":hex});
        let end = serde_json::json!({"kind":"end", "reason":"stopped", "transfers":1});
        format!("{header}\n{transfer}\n{end}\n").into_bytes()
    }

    fn screenshot() -> Image {
        let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=").unwrap();
        image("screen.png", "image/png", &bytes).unwrap()
    }

    fn stored() -> StoredEntry {
        StoredEntry { request_id: REQUEST.into(), reply_key: "ab".repeat(32), app_version: "1.4.12".into(),
            draft: draft(), created_at: 1, updated_at: 1, status: Status::Pending, number: None, url: None,
            pending_reply: None, last_reply: None }
    }

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            // Never use the real profile or a process-wide environment change.
            // CARGO_MANIFEST_DIR is the authorized D: checkout for this task.
            let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let root = workspace.join("scratch").join("feedback-client-tests");
            fs::create_dir_all(&root).unwrap();
            let directory = root.join(new_uuid().unwrap());
            fs::create_dir(&directory).unwrap();
            let resolved = directory.canonicalize().unwrap();
            assert!(resolved.starts_with(workspace.canonicalize().unwrap()));
            Self(directory)
        }
        fn history(&self) -> std::path::PathBuf { self.0.join("feedback-history.dat") }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            // A unique test-owned directory; never remove its parent or a path
            // loaded from a history record.
            if let (Ok(path), Ok(root)) = (self.0.canonicalize(),
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scratch").join("feedback-client-tests").canonicalize()) {
                if path.starts_with(&root) && path != root { let _ = fs::remove_dir_all(path); }
            }
        }
    }

    #[test]
    fn endpoint_requires_https_and_cannot_inject_credentials_paths_or_redirects() {
        assert_eq!(parse_endpoint("https://feedback.example/api/v1/"), Some(Endpoint {
            host: "feedback.example".into(), port: 443, base: "/api/v1".into() }));
        assert_eq!(parse_endpoint("https://feedback.example:8443").unwrap().port, 8443);
        for url in ["http://feedback.example", "https://a@feedback.example", "https://a:secret@b/",
            "https://feedback.example/?token=x", "https://feedback.example/#fragment", "https://feedback.example/../x",
            "https://feedback.example/%2fsecret", "https://feedback.example/a//b", "https://a:0", "https://a:65536",
            "https://a\\b", "https://a\r\nX:1", "https://a./", "https://-a.example", "", "https://"] {
            assert!(parse_endpoint(url).is_none(), "{url}");
        }
    }

    #[test]
    fn draft_bounds_unknown_fields_and_private_diagnostics_are_rejected() {
        let valid = draft(); assert!(validate_draft(&valid).is_ok());
        let mut wrong = valid.clone(); wrong.description = "short".repeat(401); assert!(validate_draft(&wrong).is_err());
        wrong = valid.clone(); wrong.device.name.push('\0'); assert!(validate_draft(&wrong).is_err());
        wrong = valid.clone(); wrong.device.family = "receiver".into(); assert!(validate_draft(&wrong).is_err());
        for empty in ["", " ", "\t"] {
            wrong = valid.clone(); wrong.device.brand = Some(empty.into()); assert!(validate_draft(&wrong).is_err());
            wrong = valid.clone(); wrong.device.model = Some(empty.into()); assert!(validate_draft(&wrong).is_err());
        }
        assert!(dimensions(10000, 6000)); assert!(!dimensions(10000, 6001));
        for private in ["serial=private", "Path: C:\\private", "alias:secret", "Authorization: Bearer key"] {
            wrong = valid.clone(); wrong.diagnostics = Some(private.into()); assert!(validate_draft(&wrong).is_err());
        }
        wrong = valid.clone(); wrong.diagnostics = Some("界".repeat(6827)); assert!(validate_draft(&wrong).is_err());
        let mut value = serde_json::to_value(valid).unwrap(); value["device"]["serial"] = "secret".into();
        assert!(serde_json::from_value::<Draft>(value).is_err());
    }

    #[test]
    fn requests_include_only_explicit_fields_and_public_history_never_contains_key() {
        let entry = stored();
        let request = serde_json::to_value(CreateRequest { schema: 1, request_id: &entry.request_id,
            reply_key: &entry.reply_key, app_version: &entry.app_version, draft: &entry.draft }).unwrap();
        assert_eq!(request["schema"], 1); assert_eq!(request["replyKey"], entry.reply_key);
        assert_eq!(request["appVersion"], "1.4.12"); assert_eq!(request["device"]["name"], "Example mouse");
        let public = serde_json::to_string(&entry.public()).unwrap();
        for private in [&entry.reply_key, "replyKey", "dataBase64", "serial", "config"] { assert!(!public.contains(private)); }
    }

    #[test]
    fn schema_one_requests_and_encrypted_history_without_capture_stay_unchanged() {
        let legacy = br#"{"category":"battery","device":{"name":"Example mouse","family":"mouse","connection":"2.4 GHz","brand":"Brand"},"description":"Battery reading is unavailable.","images":[]}"#;
        let restored: Draft = serde_json::from_slice(legacy).unwrap();
        assert!(restored.capture.is_none());
        assert_eq!(serde_json::to_vec(&restored).unwrap(), legacy);
        let mut entry = stored();
        entry.draft = restored;
        let request = serde_json::to_string(&CreateRequest { schema: 1, request_id: &entry.request_id,
            reply_key: &entry.reply_key, app_version: &entry.app_version, draft: &entry.draft }).unwrap();
        let expected = format!("{{\"schema\":1,\"requestId\":\"{REQUEST}\",\"replyKey\":\"{}\",\"appVersion\":\"1.4.12\",{}",
            entry.reply_key, std::str::from_utf8(&legacy[1..]).unwrap());
        assert_eq!(request, expected);
        let mut explicit_null = serde_json::to_value(&entry.draft).unwrap();
        explicit_null["capture"] = serde_json::Value::Null;
        assert_eq!(serde_json::from_value::<Draft>(explicit_null).unwrap(), entry.draft);
        let scratch = Scratch::new();
        let plain = serde_json::to_vec(&History { schema: 1, entries: vec![entry] }).unwrap();
        assert!(!String::from_utf8_lossy(&plain).contains("capture"));
        let mut encrypted = HISTORY_MAGIC.to_vec();
        encrypted.extend(crypt(&plain, true).unwrap());
        fs::write(scratch.history(), &encrypted).unwrap();
        let history = read_history(&scratch.history()).unwrap();
        assert!(history.entries[0].draft.capture.is_none());
        assert_eq!(serde_json::to_vec(&history).unwrap(), plain);
        save_history(&scratch.history(), &history).unwrap();
        assert_eq!(serde_json::to_vec(&read_history(&scratch.history()).unwrap()).unwrap(), plain);
    }

    #[test]
    fn capture_serializes_separately_from_images_and_keeps_diagnostics_restricted() {
        let mut entry = stored();
        entry.draft.capture = Some(capture(&raw_capture("DEADBEEF00ABFF")).unwrap());
        entry.draft.images = vec![screenshot(); 3];
        entry.draft.diagnostics = Some("Battery: unknown; charging: no".into());
        assert!(validate_draft(&entry.draft).is_ok());
        let request = serde_json::to_value(&CreateRequest { schema: 1, request_id: &entry.request_id,
            reply_key: &entry.reply_key, app_version: &entry.app_version, draft: &entry.draft }).unwrap();
        assert_eq!(request["schema"], 1);
        assert_eq!(request["images"].as_array().unwrap().len(), 3);
        assert_eq!(request["capture"], serde_json::to_value(entry.draft.capture.as_ref().unwrap()).unwrap());
        assert_eq!(request["capture"]["name"], "raw.jsonl");
        assert_eq!(request["capture"]["mimeType"], "application/x-ndjson");
        let public = serde_json::to_string(&entry.public()).unwrap();
        for private in ["capture", "dataBase64", "DEADBEEF00ABFF", &entry.reply_key] {
            assert!(!public.contains(private));
        }
        for diagnostics in ["serial:private", "path=C:\\private", "alias:private", "config:private",
            "Authorization: Bearer secret"] {
            entry.draft.diagnostics = Some(diagnostics.into());
            assert!(validate_draft(&entry.draft).is_err());
        }
    }

    #[test]
    fn pending_capture_survives_encrypted_history_and_completion_discards_attachments() {
        let scratch = Scratch::new();
        let raw = raw_capture("DEADBEEF00ABFF");
        let mut original = stored();
        original.draft.capture = Some(capture(&raw).unwrap());
        original.draft.images.push(screenshot());
        let attachment = original.draft.capture.as_ref().unwrap();
        let history = History { schema: 1, entries: vec![original.clone()] };
        save_history(&scratch.history(), &history).unwrap();
        let encrypted = fs::read(scratch.history()).unwrap();
        for private in [raw.as_slice(), attachment.data_base64.as_bytes(), original.reply_key.as_bytes()] {
            assert!(!encrypted.windows(private.len()).any(|window| window == private));
        }
        let mut restored = read_history(&scratch.history()).unwrap();
        assert_eq!(restored.entries[0].draft, original.draft);
        let retry = prepare_entry(&mut restored, original.draft.clone(), "1.4.13").unwrap();
        assert_eq!(retry.request_id, original.request_id);
        assert_eq!(retry.reply_key, original.reply_key);
        assert_eq!(retry.app_version, original.app_version);
        assert_eq!(retry.draft.capture.as_ref(), Some(attachment));
        let mut invalid = read_history(&scratch.history()).unwrap();
        invalid.entries[0].draft.capture.as_mut().unwrap().data_base64 = STANDARD.encode(b"{}\n");
        assert!(save_history(&scratch.history(), &invalid).is_err());
        assert_eq!(fs::read(scratch.history()).unwrap(), encrypted);
        assert!(complete_entry(&mut restored, &retry, CreateReceipt {
            number: 42, url: "https://example.com/issues/42".into(), reply_access: true,
        }).is_err());
        assert_eq!(restored.entries[0].draft, original.draft);
        for _attempt in 0..2 {
            complete_entry(&mut restored, &retry, CreateReceipt {
                number: 42, url: format!("{REPOSITORY}/issues/42"), reply_access: true,
            }).unwrap();
            assert!(restored.entries[0].draft.capture.is_none());
            assert!(restored.entries[0].draft.images.is_empty());
        }
        save_history(&scratch.history(), &restored).unwrap();
        let completed = read_history(&scratch.history()).unwrap();
        assert_eq!(completed.entries[0].status, Status::Completed);
        assert!(completed.entries[0].draft.capture.is_none());
        assert!(completed.entries[0].draft.images.is_empty());
        assert!(!serde_json::to_string(&completed).unwrap().contains(&attachment.data_base64));
    }

    #[test]
    fn pending_retry_identity_includes_capture_presence_and_exact_bytes() {
        let mut history = History::default();
        let mut initial = draft();
        initial.capture = Some(capture(&raw_capture("ABCD")).unwrap());
        let original = prepare_entry(&mut history, initial.clone(), "1.4.10").unwrap();
        let retry = prepare_entry(&mut history, initial.clone(), "1.4.11").unwrap();
        assert_eq!(retry.request_id, original.request_id);
        assert_eq!(retry.reply_key, original.reply_key);
        assert_eq!(retry.app_version, original.app_version);
        assert_eq!(history.entries.len(), 1);
        let mut changed = initial.clone();
        changed.capture = Some(capture(&raw_capture("ABCE")).unwrap());
        let changed = prepare_entry(&mut history, changed, "1.4.11").unwrap();
        assert_ne!(changed.request_id, original.request_id);
        assert_ne!(changed.reply_key, original.reply_key);
        let omitted = prepare_entry(&mut history, draft(), "1.4.11").unwrap();
        assert_ne!(omitted.request_id, original.request_id);
        assert_ne!(omitted.request_id, changed.request_id);
        assert_eq!(history.entries.len(), 3);
        assert_eq!(history.entries[0].draft, initial);
        assert!(history.entries.iter().all(|entry| entry.status == Status::Pending));
        assert!(validate_history(&history).is_ok());
    }

    #[test]
    fn encrypted_history_validation_preserves_pending_identity_and_rejects_corruption() {
        let initial = History { schema: 1, entries: vec![stored()] };
        assert!(validate_history(&initial).is_ok());
        let bytes = serde_json::to_vec(&initial).unwrap();
        let restored: History = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.entries[0].request_id, REQUEST);
        assert_eq!(restored.entries[0].reply_key, initial.entries[0].reply_key);
        assert_eq!(restored.entries[0].draft, initial.entries[0].draft);
        let mut bad = stored(); bad.reply_key.clear();
        assert!(validate_history(&History { schema: 1, entries: vec![bad] }).is_err());
        bad = stored(); bad.updated_at = 0;
        assert!(validate_history(&History { schema: 1, entries: vec![bad] }).is_err());
        bad = stored(); bad.draft.description = "x".repeat(2001);
        assert!(validate_history(&History { schema: 1, entries: vec![bad] }).is_err());
        let mut completed = stored(); completed.status = Status::Completed;
        completed.number = Some(42); completed.url = Some(format!("{REPOSITORY}/issues/42"));
        assert!(validate_history(&History { schema: 1, entries: vec![completed.clone()] }).is_ok());
        completed.url = Some("https://evil.example/issues/42".into());
        assert!(validate_history(&History { schema: 1, entries: vec![completed] }).is_err());
        assert!(validate_history(&History { schema: 1, entries: vec![stored(), stored()] }).is_err());
    }

    #[test]
    fn github_links_timestamps_and_receipts_are_bound_to_the_exact_issue() {
        let good = format!("{REPOSITORY}/issues/42");
        for repository in [REPOSITORY, LEGACY_REPOSITORY] {
            let url = format!("{repository}/issues/42");
            assert!(issue_url(42, &url));
            assert!(comment_url(42, 7, &format!("{url}#issuecomment-7")));
            assert!(!comment_url(42, 8, &format!("{url}#issuecomment-7")));
            for bad in [format!("{url}/"), format!("{url}?x=1"), format!("{url}#x"), url.replace("github.com", "github.com.evil.test"),
                url.replace("/42", "/042"), url.replace("youmikk", "other")] { assert!(!issue_url(42, &bad)); }
        }
        assert!(timestamp("2026-11-23T01:02:03Z")); assert!(timestamp("2024-02-29T00:00:00.123Z"));
        for date in ["2026-02-29T00:00:00Z", "2026-11-23T25:00:00Z", "2026-11-23", "2026-11-23T00:00:00Z<script>"] { assert!(!timestamp(date)); }
        assert!(valid_receipt(42, &ReplyReceipt { id: 7, url: format!("{good}#issuecomment-7"), created_at: "2026-11-23T01:02:03Z".into() }));
        assert!(!comment_url(42, 7, &format!("{REPOSITORY}/issues/43#issuecomment-7")));
        assert!(uuid(REQUEST)); assert!(!uuid("12345678-1234-1234-8234-123456789abc"));
        assert!(valid_version("1.4.12-beta.2+win")); assert!(!valid_version("1.4.12\r\nInjected:1"));
        assert!(pending(br#"{"status":"pending","retryAfter":5}"#));
        assert!(!pending(br#"{"status":"success","retryAfter":5}"#));
    }

    #[test]
    fn repository_rename_preserves_encrypted_history_and_receipt_replays() {
        let scratch = Scratch::new();
        let mut old = stored();
        old.status = Status::Completed;
        old.number = Some(42);
        old.url = Some(format!("{LEGACY_REPOSITORY}/issues/42"));
        let request = StoredReply { request_id: REQUEST.into(), body: "Additional device information".into() };
        old.last_reply = Some(FinishedReply {
            request: request.clone(),
            receipt: ReplyReceipt { id: 7, url: format!("{LEGACY_REPOSITORY}/issues/42#issuecomment-7"),
                created_at: "2026-11-23T01:02:03Z".into() },
        });
        save_history(&scratch.history(), &History { schema: 1, entries: vec![old.clone()] }).unwrap();
        let mut restored = read_history(&scratch.history()).unwrap();
        assert_eq!(restored.entries[0].reply_key, old.reply_key);
        assert_eq!(restored.entries[0].request_id, old.request_id);
        assert_eq!(restored.entries[0].public().url, Some(format!("{REPOSITORY}/issues/42")));
        for repository in [REPOSITORY, LEGACY_REPOSITORY] {
            let outcome = complete_entry(&mut restored, &old, CreateReceipt {
                number: 42, url: format!("{repository}/issues/42"), reply_access: true,
            }).unwrap();
            assert_eq!(outcome.url, Some(format!("{REPOSITORY}/issues/42")));
            let receipt = ReplyReceipt { id: 7, url: format!("{repository}/issues/42#issuecomment-7"),
                created_at: "2026-11-23T01:02:03Z".into() };
            complete_reply(&mut restored.entries[0], &old, &request, &receipt).unwrap();
        }
        save_history(&scratch.history(), &restored).unwrap();
        assert_eq!(read_history(&scratch.history()).unwrap().entries[0].reply_key, old.reply_key);
        assert!(complete_entry(&mut restored, &old, CreateReceipt {
            number: 43, url: format!("{REPOSITORY}/issues/43"), reply_access: true,
        }).is_err());
    }

    #[test]
    fn image_validation_checks_bytes_instead_of_trusting_mime_or_filename() {
        let png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=").unwrap();
        assert!(image("screen.png", "image/png", &png).is_ok());
        assert!(image("screen.jpg", "image/jpeg", &png).is_err());
        assert!(image("C:\\private\\screen.png", "image/png", &png).is_err());
        assert!(image("screen.png", "image/png", &png[..24]).is_err());
        assert!(image("screen.png", "image/png", b"<svg onload='x'/>").is_err());
        let mut bad = image("screen.png", "image/png", &png).unwrap(); bad.data_base64.push('\n');
        assert!(validate_image(&bad).is_err());
        let mut many = draft(); many.images = vec![image("screen.png", "image/png", &png).unwrap(); 4];
        assert!(validate_draft(&many).is_err());
        let mut huge = png; huge[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(!valid_image_bytes("image/png", &huge));
    }

    #[test]
    fn fallback_is_a_fixed_github_prefill_and_never_claims_attachment_upload() {
        let mut draft = draft(); draft.description = "测试反馈内容".repeat(300);
        let report = fallback(&draft).unwrap();
        assert!(report.url.starts_with(&format!("{REPOSITORY}/issues/new?title=")));
        assert!(report.url.is_ascii() && report.url.len() <= 7500);
        assert!(report.markdown.contains(&draft.description)); assert!(!report.images_omitted);
        assert!(!report.capture_omitted);
        assert!(!report.url.contains("replyKey"));
        assert_eq!(encode("中&\n"), "%E4%B8%AD%26%0A");
    }

    #[test]
    fn fallback_warns_for_each_attachment_without_embedding_capture_bytes() {
        let raw = String::from_utf8(raw_capture("DEADBEEF00ABFF")).unwrap();
        let attachment = capture(raw.as_bytes()).unwrap();
        for capture_included in [false, true] {
            for images_included in [false, true] {
                for long in [false, true] {
                    let mut draft = draft();
                    if capture_included { draft.capture = Some(attachment.clone()); }
                    if images_included { draft.images.push(screenshot()); }
                    if long { draft.description = "界".repeat(2000); }
                    let report = fallback(&draft).unwrap();
                    assert_eq!(report.capture_omitted, capture_included);
                    assert_eq!(report.images_omitted, images_included);
                    assert!(report.url.is_ascii() && report.url.len() <= MAX_PREFILL_URL);
                    for private in [raw.as_str(), attachment.data_base64.as_str(), "DEADBEEF00ABFF", "dataBase64"] {
                        assert!(!report.markdown.contains(private));
                        assert!(!report.url.contains(private));
                        assert!(!report.url.contains(&encode(private)));
                    }
                    if capture_included {
                        assert!(report.markdown.contains("原始采集文件") || report.markdown.contains("raw capture files"));
                        assert!(report.url.contains(&encode("原始采集文件")) || report.url.contains(&encode("raw capture files")));
                    }
                    if images_included || capture_included {
                        assert!(report.markdown.contains("手动添加") || report.markdown.contains("manually on GitHub"));
                    }
                    let serialized = serde_json::to_value(&report).unwrap();
                    assert_eq!(serialized["captureOmitted"], capture_included);
                    assert_eq!(serialized["imagesOmitted"], images_included);
                }
            }
        }
    }

    #[test]
    fn fallback_budget_handles_maximum_unicode_fields_without_losing_full_report() {
        fn decode(value: &str) -> String {
            let mut bytes = Vec::new();
            let mut position = 0;
            while position < value.len() {
                if value.as_bytes()[position] == b'%' {
                    bytes.push(u8::from_str_radix(&value[position + 1..position + 3], 16).unwrap());
                    position += 3;
                } else {
                    bytes.push(value.as_bytes()[position]);
                    position += 1;
                }
            }
            String::from_utf8(bytes).unwrap()
        }
        for pattern in ["🧪", "🧪界&<>\\_*%?="] {
            let fill = |count| pattern.chars().cycle().take(count).collect::<String>();
            let mut draft = draft();
            draft.device.name = fill(120);
            draft.device.connection = fill(80);
            draft.device.brand = Some(fill(80));
            draft.device.model = Some(fill(120));
            draft.description = fill(2000);
            draft.diagnostics = Some("量".repeat(6000));
            let report = fallback(&draft).unwrap();
            assert!(report.url.is_ascii() && report.url.len() <= MAX_PREFILL_URL);
            let query = report.url.strip_prefix(&format!("{REPOSITORY}/issues/new?title=")).unwrap();
            let (title, body) = query.split_once("&body=").unwrap();
            assert!(!body.contains('&'));
            assert!(decode(title).starts_with('['));
            assert!(decode(body).contains(&message("请在提交前粘贴完整报告和诊断摘要。",
                "Paste the complete report and diagnostic summary before submitting.")));
            assert!(report.markdown.contains(&draft.description));
            assert!(report.markdown.contains(draft.diagnostics.as_ref().unwrap()));
            assert!(report.markdown.contains(&markdown_label(&draft.device.name)));
            assert!(report.markdown.contains(&markdown_label(&draft.device.connection)));
        }
        for budget in 0..30 {
            let prefix = encoded_prefix("🧪中A&", budget);
            assert!(prefix.len() <= budget);
            assert!("🧪中A&".starts_with(&decode(&prefix)));
        }
    }

    #[test]
    fn retrying_pending_draft_reuses_both_identifiers_and_completion_is_idempotent() {
        let original = stored();
        let mut history = History { schema: 1, entries: vec![original.clone()] };
        let retry = prepare_entry(&mut history, original.draft.clone(), "1.4.13").unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(retry.request_id, original.request_id);
        assert_eq!(retry.reply_key, original.reply_key);
        assert_eq!(retry.app_version, original.app_version);
        // A timeout or pending receipt makes no state transition. Recreate the
        // saved file state in memory as happens after a process restart.
        let restored: History = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        assert_eq!(restored.entries[0].status, Status::Pending);
        for _ in 0..2 {
            let receipt = CreateReceipt { number: 42, url: format!("{REPOSITORY}/issues/42"), reply_access: true };
            assert_eq!(complete_entry(&mut history, &retry, receipt).unwrap().status, Status::Completed);
        }
        let conflict = CreateReceipt { number: 43, url: format!("{REPOSITORY}/issues/43"), reply_access: true };
        assert!(complete_entry(&mut history, &retry, conflict).is_err());
        assert_eq!(history.entries[0].number, Some(42));
        assert!(validate_history(&history).is_ok());
    }

    #[test]
    fn failed_reply_reuses_saved_body_and_success_cannot_be_recorded_for_another_issue() {
        let mut entry = stored(); entry.status = Status::Completed;
        entry.number = Some(42); entry.url = Some(format!("{REPOSITORY}/issues/42"));
        entry.pending_reply = Some(StoredReply { request_id: "12345678-1234-4234-9234-123456789abc".into(), body: "More details".into() });
        let original = entry.clone();
        prepare_reply(&mut entry, "More details").unwrap();
        assert_eq!(entry.pending_reply.as_ref().unwrap().request_id, original.pending_reply.as_ref().unwrap().request_id);
        assert!(prepare_reply(&mut entry, "Different details").is_err());
        let pending = original.pending_reply.as_ref().unwrap();
        let mut receipt = ReplyReceipt { id: 7, url: format!("{REPOSITORY}/issues/43#issuecomment-7"),
            created_at: "2026-11-23T00:00:00Z".into() };
        assert!(complete_reply(&mut entry, &original, pending, &receipt).is_err());
        assert!(entry.pending_reply.is_some());
        receipt.url = format!("{REPOSITORY}/issues/42#issuecomment-7");
        complete_reply(&mut entry, &original, pending, &receipt).unwrap();
        complete_reply(&mut entry, &original, pending, &receipt).unwrap();
        assert!(entry.pending_reply.is_none());
        assert_eq!(entry.last_reply.unwrap().receipt.id, 7);
    }

    #[test]
    fn same_text_after_a_completed_reply_starts_a_new_operation() {
        let mut entry = stored(); entry.status = Status::Completed;
        entry.number = Some(42); entry.url = Some(format!("{REPOSITORY}/issues/42"));
        entry.pending_reply = Some(StoredReply { request_id: "12345678-1234-4234-9234-123456789abc".into(), body: "还是不行".into() });
        let first = entry.clone();
        let first_pending = first.pending_reply.as_ref().unwrap();
        let receipt = ReplyReceipt { id: 7, url: format!("{REPOSITORY}/issues/42#issuecomment-7"),
            created_at: "2026-11-23T00:00:00Z".into() };
        complete_reply(&mut entry, &first, first_pending, &receipt).unwrap();
        let mut reopened: StoredEntry = serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
        prepare_reply(&mut reopened, "还是不行").unwrap();
        let second = reopened.pending_reply.as_ref().unwrap().clone();
        assert_eq!(second.body, first_pending.body);
        assert_ne!(second.request_id, first_pending.request_id);
        assert!(uuid(&second.request_id));
        // A pending retry keeps the new UUID. A late receipt from the first
        // operation must not mark the second operation as completed.
        prepare_reply(&mut reopened, "还是不行").unwrap();
        complete_reply(&mut reopened, &first, first_pending, &receipt).unwrap();
        assert_eq!(reopened.pending_reply.as_ref().unwrap().request_id, second.request_id);
        assert_eq!(reopened.last_reply.as_ref().unwrap().receipt.id, 7);
        let second_stored = reopened.clone();
        let second_receipt = ReplyReceipt { id: 8, url: format!("{REPOSITORY}/issues/42#issuecomment-8"),
            created_at: "2026-11-23T01:00:00Z".into() };
        complete_reply(&mut reopened, &second_stored, &second, &second_receipt).unwrap();
        assert!(reopened.pending_reply.is_none());
        assert_eq!(reopened.last_reply.unwrap().receipt.id, 8);
    }

    #[test]
    fn only_explicit_terminal_rejection_releases_the_exact_pending_reply() {
        let mut entry = stored(); entry.status = Status::Completed;
        entry.number = Some(42); entry.url = Some(format!("{REPOSITORY}/issues/42"));
        entry.pending_reply = Some(StoredReply { request_id: "12345678-1234-4234-9234-123456789abc".into(), body: "Original reply".into() });
        let original = entry.clone();
        let pending = original.pending_reply.as_ref().unwrap();
        let terminal = br#"{"error":{"code":"github_rejected","retryable":false,"message":"not displayed"}}"#;
        for (status, bytes) in [(503, terminal.as_slice()), (202, terminal.as_slice()), (0, &[][..]),
            (502, br#"{"error":{"code":"github_rejected","retryable":true}}"#.as_slice()),
            (502, br#"{"error":{"code":"unknown","retryable":false}}"#.as_slice()),
            (502, br#"{"error":{"code":"github_rejected"}}"#.as_slice()), (502, b"bad json".as_slice())] {
            assert!(!clear_rejected_reply(&mut entry, &original, pending, status, bytes));
            assert_eq!(entry.pending_reply.as_ref().unwrap().request_id, pending.request_id);
            assert!(prepare_reply(&mut entry, "Changed reply").is_err());
        }
        let mut unrelated = original.clone();
        unrelated.pending_reply.as_mut().unwrap().request_id = "12345678-1234-4234-a234-123456789abc".into();
        assert!(!clear_rejected_reply(&mut unrelated, &original, pending, 502, terminal));
        assert!(unrelated.pending_reply.is_some());
        assert!(clear_rejected_reply(&mut entry, &original, pending, 502, terminal));
        assert!(entry.pending_reply.is_none());
        prepare_reply(&mut entry, "Changed reply").unwrap();
        assert_eq!(entry.pending_reply.as_ref().unwrap().body, "Changed reply");
        assert_ne!(entry.pending_reply.unwrap().request_id, pending.request_id);
        let mut oversized = terminal.to_vec(); oversized.resize(4097, b' ');
        assert!(!rejected(502, &oversized));
    }

    #[test]
    fn dpapi_roundtrip_keeps_request_keys_encrypted_and_rejects_tampering() {
        let entry = stored();
        let plain = serde_json::to_vec(&History { schema: 1, entries: vec![entry.clone()] }).unwrap();
        let encrypted = crypt(&plain, true).unwrap();
        assert_ne!(encrypted, plain);
        assert!(!encrypted.windows(entry.reply_key.len()).any(|w| w == entry.reply_key.as_bytes()));
        assert!(!encrypted.windows(entry.request_id.len()).any(|w| w == entry.request_id.as_bytes()));
        assert_eq!(crypt(&encrypted, false).unwrap(), plain);
        let mut tampered = encrypted.clone();
        let end = tampered.len() - 1; tampered[end] ^= 1;
        assert!(crypt(&tampered, false).is_err());
        assert!(crypt(&encrypted[..encrypted.len() / 2], false).is_err());
    }

    #[test]
    fn atomic_history_persistence_recovers_pending_request_and_never_writes_plaintext() {
        let scratch = Scratch::new();
        let path = scratch.history();
        assert!(read_history(&path).unwrap().entries.is_empty());
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
        let initial = stored();
        let mut history = History { schema: 1, entries: vec![initial.clone()] };
        let lock = lock_history(&path).unwrap();
        save_history(&path, &history).unwrap();
        let raw = fs::read(&path).unwrap();
        assert!(raw.starts_with(HISTORY_MAGIC));
        assert!(!raw.windows(initial.reply_key.len()).any(|w| w == initial.reply_key.as_bytes()));
        let restored = read_history(&path).unwrap();
        assert_eq!(restored.entries[0].request_id, initial.request_id);
        assert_eq!(restored.entries[0].reply_key, initial.reply_key);
        assert_eq!(restored.entries[0].draft, initial.draft);
        assert_eq!(restored.entries[0].status, Status::Pending);
        complete_entry(&mut history, &initial, CreateReceipt { number: 42,
            url: format!("{REPOSITORY}/issues/42"), reply_access: true }).unwrap();
        save_history(&path, &history).unwrap();
        assert_eq!(read_history(&path).unwrap().entries[0].status, Status::Completed);
        let good_bytes = fs::read(&path).unwrap();
        history.entries[0].reply_key.clear();
        assert!(save_history(&path, &history).is_err());
        assert_eq!(fs::read(&path).unwrap(), good_bytes);
        drop(lock);
        let mut files: Vec<_> = fs::read_dir(&scratch.0).unwrap().map(|e| e.unwrap().file_name()).collect();
        files.sort();
        assert_eq!(files, [std::ffi::OsString::from("feedback-history.dat"), std::ffi::OsString::from("feedback-history.lock")]);
    }

    #[test]
    fn history_lock_excludes_competing_threads_and_reopens_after_release() {
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = Scratch::new();
        let path = scratch.history();
        let first = lock_history(&path).unwrap();
        let lock = scratch.0.join("feedback-history.lock");
        let concurrent = std::thread::spawn(move || {
            OpenOptions::new().read(true).write(true).create(true).share_mode(0).open(lock)
                .unwrap_err().raw_os_error()
        }).join().unwrap();
        assert_eq!(concurrent, Some(32));
        drop(first);
        let second_path = path.clone();
        assert!(std::thread::spawn(move || lock_history(&second_path).is_ok()).join().unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn history_lock_remains_compatible_with_the_previous_delete_on_close_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = Scratch::new();
        let path = scratch.history();
        let lock = scratch.0.join("feedback-history.lock");
        let legacy = || OpenOptions::new().read(true).write(true).create(true).share_mode(0)
            .access_mode(winapi::um::winnt::GENERIC_READ | winapi::um::winnt::GENERIC_WRITE | winapi::um::winnt::DELETE)
            .custom_flags(winapi::um::winbase::FILE_FLAG_DELETE_ON_CLOSE).open(&lock);
        let first = lock_history(&path).unwrap();
        assert_eq!(legacy().unwrap_err().raw_os_error(), Some(32));
        drop(first);
        let old = legacy().unwrap();
        assert_eq!(OpenOptions::new().read(true).write(true).create(true).share_mode(0)
            .open(&lock).unwrap_err().raw_os_error(), Some(32));
        drop(old);
        assert!(!lock.exists());
        let reopened = lock_history(&path).unwrap();
        drop(reopened);
        assert!(lock.exists());
    }

    #[test]
    fn encrypted_history_creates_missing_directories_without_a_delete_on_close_lock() {
        let scratch = Scratch::new();
        let path = scratch.0.join("new").join("nested").join("feedback-history.dat");
        let _lock = lock_history(&path).unwrap();
        assert!(path.parent().unwrap().exists());
        let history = History { schema: 1, entries: vec![stored()] };
        save_history(&path, &history).unwrap();
        assert_eq!(read_history(&path).unwrap().entries[0].request_id, REQUEST);
        assert!(fs::read(&path).unwrap().starts_with(HISTORY_MAGIC));
        assert!(path.parent().unwrap().join("feedback-history.lock").exists());
    }

    #[test]
    fn storage_errors_identify_the_failed_stage_and_windows_error() {
        let error = storage_io_error(("安全替换本地记录", "Replace local history"), std::io::Error::from_raw_os_error(32));
        assert!(error.contains(&message("安全替换本地记录", "Replace local history")));
        assert!(error.contains("32"));
    }

    #[test]
    fn failed_atomic_replacement_preserves_last_saved_pending_request() {
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = Scratch::new();
        let path = scratch.history();
        let mut original = stored();
        original.draft.capture = Some(capture(&raw_capture("DEADBEEF00ABFF")).unwrap());
        original.draft.images.push(screenshot());
        let mut history = History { schema: 1, entries: vec![original.clone()] };
        let _lock = lock_history(&path).unwrap();
        save_history(&path, &history).unwrap();
        // Deny FILE_SHARE_DELETE to simulate an antivirus or another reader
        // preventing the final rename after the encrypted temp file is flushed.
        let busy = OpenOptions::new().read(true).share_mode(winapi::um::winnt::FILE_SHARE_READ).open(&path).unwrap();
        complete_entry(&mut history, &original, CreateReceipt { number: 42,
            url: format!("{REPOSITORY}/issues/42"), reply_access: true }).unwrap();
        assert!(save_history(&path, &history).is_err());
        drop(busy);
        let restored = read_history(&path).unwrap();
        assert_eq!(restored.entries[0].status, Status::Pending);
        assert_eq!(restored.entries[0].request_id, original.request_id);
        assert_eq!(restored.entries[0].reply_key, original.reply_key);
        assert_eq!(restored.entries[0].draft, original.draft);
        assert!(fs::read_dir(&scratch.0).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
        save_history(&path, &history).unwrap();
        let completed = read_history(&path).unwrap();
        assert_eq!(completed.entries[0].number, Some(42));
        assert!(completed.entries[0].draft.capture.is_none());
        assert!(completed.entries[0].draft.images.is_empty());
    }
}
