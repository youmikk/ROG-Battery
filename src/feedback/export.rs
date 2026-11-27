use super::*;
use std::path::PathBuf;

pub fn export_draft(draft: &Draft, root: &Path) -> Result<PathBuf, String> {
    local_files::register_locked(&history_path(), || save(draft, crate::gui::APP_VERSION, None, None, root))
}

pub fn export_local(request_id: &str, root: &Path) -> Result<PathBuf, String> {
    if !uuid(request_id) { return Err(invalid()); }
    with_locked_history_at(&history_path(), true, |path, history| {
        let entry = history.entries.iter().find(|entry| entry.request_id == request_id).ok_or_else(damaged)?;
        local_files::register(path, || save(&entry.draft, &entry.app_version, Some(&entry.public()),
            entry.pending_reply.as_ref().map(|reply| reply.body.as_str()), root))
    })
}

pub(super) fn save(draft: &Draft, version: &str, receipt: Option<&Entry>, followup: Option<&str>, root: &Path) -> Result<local_files::Bundle, String> {
    save_with(draft, version, receipt, followup, root, |file, bytes| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn save_with(draft: &Draft, version: &str, receipt: Option<&Entry>, followup: Option<&str>, root: &Path,
    write: impl FnMut(&mut File, &[u8]) -> std::io::Result<()>) -> Result<local_files::Bundle, String> {
    let report = fallback_version(draft, version)?.markdown;
    let folder = format!("X-Battery-feedback-{}-{}", now(), new_uuid()?);
    let mut contents = Vec::new();
    let mut attachments = Vec::new();
    for (index, image) in draft.images.iter().enumerate() {
        let name = local_files::image_name(index, &image.mime_type)?;
        let bytes = STANDARD.decode(&image.data_base64).map_err(|_| invalid())?;
        contents.push((name.clone(), bytes));
        attachments.push(name);
    }
    if let Some(capture) = &draft.capture {
        let bytes = STANDARD.decode(&capture.data_base64).map_err(|_| invalid())?;
        contents.push(("raw.jsonl".into(), bytes));
        attachments.push("raw.jsonl".into());
    }
    if let Some(body) = followup {
        contents.push(("follow-up.txt".into(), body.as_bytes().to_vec()));
        attachments.push("follow-up.txt".into());
    }
    let manifest = serde_json::json!({
        "schema": 1, "appVersion": version, "category": draft.category, "device": draft.device,
        "description": draft.description, "diagnostics": draft.diagnostics, "attachments": attachments,
        "receipt": receipt,
    });
    let manifest = serde_json::to_vec_pretty(&manifest).map_err(|_| invalid())?;
    contents.push(("feedback.json".into(), manifest));
    contents.push(("report.md".into(), report.into_bytes()));
    let network = crate::feedback::software::network_summary();
    contents.push(("network-summary.txt".into(), network.into_bytes()));
    let binding = receipt.map(|entry| local_files::Binding::Request(entry.request_id.clone()))
        .map_or_else(|| local_files::Binding::draft(draft), Ok)?;
    local_files::write_bundle(root, &folder, binding, contents, write)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: u8, app_version: String, category: String, device: Device, description: String,
    diagnostics: Option<String>, attachments: Vec<String>, receipt: Option<Entry>,
}

pub(super) fn legacy_binding(contents: &std::collections::BTreeMap<String, Vec<u8>>, history: &History)
    -> Result<Option<local_files::Binding>, String> {
    let Some(bytes) = contents.get("feedback.json") else { return Ok(None); };
    let Ok(manifest) = serde_json::from_slice::<Manifest>(bytes) else { return Ok(None); };
    if manifest.schema != 1 || !valid_version(&manifest.app_version) { return Ok(None); }
    let mut names: BTreeSet<_> = ["feedback.json", "report.md", "network-summary.txt"].into_iter().collect();
    for name in &manifest.attachments {
        if !names.insert(name) { return Ok(None); }
    }
    if contents.len() != names.len() || !contents.keys().all(|name| names.contains(name.as_str())) { return Ok(None); }
    let mut draft = Draft { category: manifest.category, device: manifest.device,
        description: manifest.description, diagnostics: manifest.diagnostics, images: Vec::new(), capture: None };
    for name in &manifest.attachments {
        let Some(bytes) = contents.get(name) else { return Ok(None); };
        if name == "raw.jsonl" {
            let Ok(capture) = capture(bytes) else { return Ok(None); };
            draft.capture = Some(capture);
        } else if name == "follow-up.txt" {
            let Ok(body) = std::str::from_utf8(bytes) else { return Ok(None); };
            if !text(body, 1, 2000, true) { return Ok(None); }
        } else {
            let mime = match name.rsplit_once('.').map(|(_, suffix)| suffix) {
                Some("png") => "image/png", Some("jpg") => "image/jpeg", Some("webp") => "image/webp", _ => return Ok(None),
            };
            if name != &local_files::image_name(draft.images.len(), mime)? { return Ok(None); }
            let Ok(image) = image(name, mime, bytes) else { return Ok(None); };
            draft.images.push(image);
        }
    }
    let Ok(report) = fallback_version(&draft, &manifest.app_version) else { return Ok(None); };
    if contents.get("report.md").map(Vec::as_slice) != Some(report.markdown.as_bytes()) { return Ok(None); }
    if !contents.get("network-summary.txt").is_some_and(|bytes| bytes.starts_with(b"Latest network failure")) { return Ok(None); }
    if let Some(receipt) = manifest.receipt {
        let Some(entry) = history.entries.iter().find(|entry| entry.request_id == receipt.request_id) else { return Ok(None); };
        if draft.category != entry.draft.category || draft.device != entry.draft.device
            || draft.description != entry.draft.description || draft.diagnostics != entry.draft.diagnostics
            || manifest.app_version != entry.app_version || receipt.category != draft.category
            || receipt.device != draft.device || receipt.description != draft.description {
            return Ok(None);
        }
        return Ok(Some(local_files::Binding::Request(receipt.request_id)));
    }
    Ok(Some(local_files::Binding::draft(&draft)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{draft, screenshot, raw_capture, Scratch};

    #[test]
    fn offline_export_keeps_attachments_and_original_version_without_followup_credentials() {
        let scratch = Scratch::new();
        let mut draft = draft();
        let screenshot = screenshot();
        let original = STANDARD.decode(&screenshot.data_base64).unwrap();
        draft.images.push(screenshot);
        let raw = raw_capture("AABBCC");
        draft.capture = Some(capture(&raw).unwrap());
        let folder = save(&draft, "1.4.9", None, Some("Pending follow-up"), &scratch.0).unwrap().path;
        assert_eq!(fs::read(folder.join("screenshot-1.png")).unwrap(), original);
        assert_eq!(fs::read(folder.join("raw.jsonl")).unwrap(), raw);
        assert_eq!(fs::read_to_string(folder.join("follow-up.txt")).unwrap(), "Pending follow-up");
        let manifest = fs::read_to_string(folder.join("feedback.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(value["appVersion"], "1.4.9");
        assert_eq!(value["description"], draft.description);
        assert!(!manifest.contains("replyKey"));
        assert!(fs::read_to_string(folder.join("report.md")).unwrap().starts_with("X-Battery 1.4.9\n"));
    }

    #[test]
    fn failed_export_cleans_only_its_new_directory() {
        let scratch = Scratch::new();
        let marker = scratch.0.join("keep.txt");
        fs::write(&marker, b"keep").unwrap();
        assert!(save_with(&draft(), "1.4.9", None, None, &scratch.0, |_, _|
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))).is_err());
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
        assert_eq!(fs::read(marker).unwrap(), b"keep");
    }
}
