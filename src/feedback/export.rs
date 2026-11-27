use super::*;
use std::path::PathBuf;

pub fn export_draft(draft: &Draft, root: &Path) -> Result<PathBuf, String> {
    save(draft, crate::gui::APP_VERSION, None, root)
}

pub fn export_local(request_id: &str, root: &Path) -> Result<PathBuf, String> {
    if !uuid(request_id) { return Err(invalid()); }
    let entry = with_history(false, |history| history.entries.iter()
        .find(|entry| entry.request_id == request_id).cloned().ok_or_else(damaged))?;
    save(&entry.draft, &entry.app_version, Some(&entry.public()), root)
}

fn save(draft: &Draft, version: &str, receipt: Option<&Entry>, root: &Path) -> Result<PathBuf, String> {
    save_with(draft, version, receipt, root, |path, bytes| {
        let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn save_with(draft: &Draft, version: &str, receipt: Option<&Entry>, root: &Path,
    mut write: impl FnMut(&Path, &[u8]) -> std::io::Result<()>) -> Result<PathBuf, String> {
    let report = fallback_version(draft, version)?.markdown;
    let folder = root.join(format!("X-Battery-feedback-{}-{}", now(), new_uuid()?));
    fs::create_dir(&folder).map_err(|error| storage_io_error(("创建导出目录", "Create export directory"), error))?;
    let result = (|| {
        let mut attachments = Vec::new();
        for (index, image) in draft.images.iter().enumerate() {
            let extension = match image.mime_type.as_str() {
                "image/png" => "png", "image/jpeg" => "jpg", "image/webp" => "webp", _ => return Err(invalid()),
            };
            let name = format!("screenshot-{}.{}", index + 1, extension);
            let bytes = STANDARD.decode(&image.data_base64).map_err(|_| invalid())?;
            write(&folder.join(&name), &bytes).map_err(|error| storage_io_error(("导出截图", "Export screenshot"), error))?;
            attachments.push(name);
        }
        if let Some(capture) = &draft.capture {
            let bytes = STANDARD.decode(&capture.data_base64).map_err(|_| invalid())?;
            write(&folder.join("raw.jsonl"), &bytes).map_err(|error| storage_io_error(("导出采集", "Export capture"), error))?;
            attachments.push("raw.jsonl".into());
        }
        let manifest = serde_json::json!({
            "schema": 1, "appVersion": version, "category": draft.category, "device": draft.device,
            "description": draft.description, "diagnostics": draft.diagnostics, "attachments": attachments,
            "receipt": receipt,
        });
        let manifest = serde_json::to_vec_pretty(&manifest).map_err(|_| invalid())?;
        write(&folder.join("feedback.json"), &manifest).map_err(|error| storage_io_error(("导出反馈", "Export feedback"), error))?;
        write(&folder.join("report.md"), report.as_bytes()).map_err(|error| storage_io_error(("导出说明", "Export report"), error))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    result.map(|()| folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{draft, screenshot, Scratch};

    #[test]
    fn offline_export_keeps_attachments_and_original_version_without_followup_credentials() {
        let scratch = Scratch::new();
        let mut draft = draft();
        let screenshot = screenshot();
        let original = STANDARD.decode(&screenshot.data_base64).unwrap();
        draft.images.push(screenshot);
        let folder = save(&draft, "1.4.9", None, &scratch.0).unwrap();
        assert_eq!(fs::read(folder.join("screenshot-1.png")).unwrap(), original);
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
        assert!(save_with(&draft(), "1.4.9", None, &scratch.0, |_, _|
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))).is_err());
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
        assert_eq!(fs::read(marker).unwrap(), b"keep");
    }
}
