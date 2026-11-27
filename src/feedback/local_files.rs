use super::*;
use std::{io, mem, os::windows::{fs::{MetadataExt, OpenOptionsExt}, io::AsRawHandle},
    path::{Component, PathBuf, Prefix}};
use winapi::{shared::bcrypt::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE}, um::{
    fileapi::{GetFileInformationByHandle, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO},
    minwinbase::FileDispositionInfo,
    winbase::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
    winnt::{DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
        GENERIC_READ, GENERIC_WRITE},
}};

const MAX_BUNDLES: usize = 1024;
const MAX_CATALOG: usize = 2 * 1024 * 1024;
const MAX_CAPTURE_BYTES: usize = 544 * 1024;
const CATALOG_MAGIC: &[u8] = b"RBFL\x01\0\0\0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fingerprint { bytes: u64, sha256: [u8; 32] }

impl Fingerprint {
    fn of(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_REQUEST { return Err(invalid()); }
        let mut sha256 = [0; 32];
        let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null_mut(), 0,
            bytes.as_ptr() as *mut u8, bytes.len() as u32, sha256.as_mut_ptr(), 32) };
        if status < 0 { return Err(storage_error()); }
        Ok(Self { bytes: bytes.len() as u64, sha256 })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity { draft: Fingerprint, capture: Option<Fingerprint> }

impl Identity {
    pub(super) fn of(draft: &Draft) -> Result<Self, String> {
        let mut normalized = draft.clone();
        normalized.diagnostics = None;
        for (index, image) in normalized.images.iter_mut().enumerate() {
            image.name = image_name(index, &image.mime_type)?;
        }
        let capture = draft.capture.as_ref().map(|capture|
            STANDARD.decode(&capture.data_base64).map_err(|_| invalid())
                .and_then(|bytes| Fingerprint::of(&bytes))).transpose()?;
        Ok(Self { draft: Fingerprint::of(&serde_json::to_vec(&normalized).map_err(|_| invalid())?)?, capture })
    }
}

pub(super) fn image_name(index: usize, mime: &str) -> Result<String, String> {
    let extension = match mime {
        "image/png" => "png", "image/jpeg" => "jpg", "image/webp" => "webp", _ => return Err(invalid()),
    };
    Ok(format!("screenshot-{}.{extension}", index + 1))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Binding { Request(String), Draft(Fingerprint), Capture(Fingerprint) }

impl Binding {
    pub(super) fn draft(draft: &Draft) -> Result<Self, String> { Ok(Self::Draft(Identity::of(draft)?.draft)) }
    pub(crate) fn capture(raw: &[u8]) -> Result<Self, String> {
        if raw.len() > MAX_CAPTURE_BYTES { return Err(invalid()); }
        Ok(Self::Capture(Fingerprint::of(raw)?))
    }

    fn matches(&self, entry: &StoredEntry) -> Result<bool, String> {
        if let Self::Request(request) = self { return Ok(request == &entry.request_id); }
        let identity = match &entry.local_identity {
            Some(identity) => identity.clone(),
            None if entry.status == Status::Completed => return Ok(false),
            None => Identity::of(&entry.draft)?,
        };
        Ok(match self {
            Self::Draft(fingerprint) => fingerprint == &identity.draft,
            Self::Capture(fingerprint) => identity.capture.as_ref() == Some(fingerprint),
            Self::Request(_) => unreachable!(),
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity { volume: u32, index: u64, created: u64 }

fn file_identity(file: &File, directory: bool) -> io::Result<FileIdentity> {
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || metadata.is_dir() != directory || (!directory && !metadata.is_file()) {
        return Err(unsafe_path());
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if !directory && info.nNumberOfLinks != 1 { return Err(unsafe_path()); }
    Ok(FileIdentity { volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        created: (u64::from(info.ftCreationTime.dwHighDateTime) << 32) | u64::from(info.ftCreationTime.dwLowDateTime) })
}

fn unsafe_path() -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, "Unverified local feedback file") }

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.ends_with(['.', ' ']) && !name.contains(['/', '\\', ':', '\0'])
        && !matches!(name, "." | "..")
}

fn safe_absolute(path: &Path) -> io::Result<PathBuf> {
    if path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
        return Err(unsafe_path());
    }
    let absolute = std::path::absolute(path)?;
    if !matches!(absolute.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))) {
        return Err(unsafe_path());
    }
    for part in absolute.components() {
        if let Component::Normal(name) = part {
            if !name.to_str().is_some_and(safe_name) { return Err(unsafe_path()); }
        }
    }
    Ok(absolute)
}

fn directory_handles(path: &Path, delete_leaf: bool) -> io::Result<Vec<File>> {
    let absolute = safe_absolute(path)?;
    let mut ancestors: Vec<_> = absolute.ancestors().collect();
    ancestors.reverse();
    let mut handles = Vec::new();
    for ancestor in ancestors {
        let file = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES
            | if delete_leaf && ancestor == absolute { DELETE } else { 0 })
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(ancestor)?;
        file_identity(&file, true)?;
        handles.push(file);
    }
    Ok(handles)
}

pub(crate) fn create_directory(path: &Path) -> Result<(), String> {
    let absolute = safe_absolute(path).map_err(failure)?;
    let mut ancestors: Vec<_> = absolute.ancestors().collect();
    ancestors.reverse();
    let mut guards = Vec::new();
    for ancestor in ancestors {
        let open = || OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(ancestor);
        let file = match open() {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(ancestor) {
                    Ok(()) => (),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(error) => return Err(failure(error)),
                }
                open().map_err(failure)?
            }
            Err(error) => return Err(failure(error)),
        };
        file_identity(&file, true).map_err(failure)?;
        guards.push(file);
    }
    Ok(())
}

fn open_file(path: &Path, delete: bool) -> io::Result<File> {
    let file = OpenOptions::new().access_mode(GENERIC_READ | if delete { DELETE } else { 0 })
        .share_mode(FILE_SHARE_READ).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
    file_identity(&file, false)?;
    Ok(file)
}

fn read_bytes(file: &mut File, maximum: usize) -> io::Result<Vec<u8>> {
    if file.metadata()?.len() > maximum as u64 { return Err(unsafe_path()); }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum { return Err(unsafe_path()); }
    Ok(bytes)
}

fn mark_deleted(file: &File) -> io::Result<()> {
    let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: 1 };
    if unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), FileDispositionInfo,
        (&mut disposition as *mut FILE_DISPOSITION_INFO).cast(), mem::size_of_val(&disposition) as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn failure(error: io::Error) -> String {
    format!("{}: {error}", message("本地文件操作失败", "Could not update local files"))
}

pub(super) fn remove_history(path: &Path) -> Result<(), String> {
    let Some(parent) = path.parent() else { return Err(invalid()); };
    let _parents = match directory_handles(parent, false) {
        Ok(handles) => handles,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(error)),
    };
    match open_file(path, true) {
        Ok(file) => mark_deleted(&file).map_err(failure),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failure(error)),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedFile { name: String, identity: FileIdentity, fingerprint: Fingerprint }

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bundle {
    pub(crate) path: PathBuf,
    identity: FileIdentity,
    binding: Binding,
    files: Vec<OwnedFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    deleted_for: Vec<String>,
}

fn allowed_file(name: &str, capture: bool) -> bool {
    if capture { return matches!(name, "raw.jsonl" | "summary.txt"); }
    matches!(name, "feedback.json" | "report.md" | "network-summary.txt" | "raw.jsonl" | "follow-up.txt")
        || (0..3).any(|index| ["image/png", "image/jpeg", "image/webp"].iter()
            .any(|mime| image_name(index, mime).is_ok_and(|expected| expected == name)))
}

fn bundle_name(name: &str, capture: bool) -> bool {
    if capture {
        return name.strip_prefix("capture-").is_some_and(|suffix| {
            let parts: Vec<_> = suffix.split('-').collect();
            parts.len() == 3 && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        });
    }
    name.strip_prefix("X-Battery-feedback-").and_then(|suffix| suffix.split_once('-'))
        .is_some_and(|(time, id)| !time.is_empty() && time.bytes().all(|byte| byte.is_ascii_digit()) && uuid(id))
}

impl Bundle {
    fn validate(&self) -> Result<(), String> {
        let capture = matches!(self.binding, Binding::Capture(_));
        let mut names = BTreeSet::new();
        if !self.path.is_absolute() || safe_absolute(&self.path).is_err()
            || !self.path.file_name().and_then(|name| name.to_str()).is_some_and(|name| bundle_name(name, capture))
            || self.files.is_empty() || self.files.len() > 8
            || self.files.iter().any(|file| !allowed_file(&file.name, capture)
                || file.fingerprint.bytes > MAX_IMAGE as u64 || !names.insert(&file.name))
            || self.deleted_for.len() > MAX_ENTRIES || self.deleted_for.iter().any(|id| !uuid(id)) {
            return Err(damaged());
        }
        if let Binding::Request(request) = &self.binding {
            if !uuid(request) { return Err(damaged()); }
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        self.validate()?;
        let handles = match directory_handles(&self.path, true) {
            Ok(handles) => handles,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(failure(error)),
        };
        let directory = handles.last().ok_or_else(damaged)?;
        if file_identity(directory, true).map_err(failure)? != self.identity { return Err(failure(unsafe_path())); }
        let mut verified = Vec::new();
        for expected in &self.files {
            let mut file = match open_file(&self.path.join(&expected.name), true) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(failure(error)),
            };
            if file_identity(&file, false).map_err(failure)? != expected.identity
                || Fingerprint::of(&read_bytes(&mut file, MAX_IMAGE).map_err(failure)?)? != expected.fingerprint {
                return Err(failure(unsafe_path()));
            }
            verified.push(file);
        }
        for file in &verified { mark_deleted(file).map_err(failure)?; }
        drop(verified);
        match mark_deleted(directory) {
            Ok(()) => Ok(()),
            Err(error) if error.raw_os_error() == Some(145) => Ok(()),
            Err(error) => Err(failure(error)),
        }
    }
}

pub(crate) fn write_bundle(root: &Path, name: &str, binding: Binding, contents: Vec<(String, Vec<u8>)>,
    mut write: impl FnMut(&mut File, &[u8]) -> io::Result<()>) -> Result<Bundle, String> {
    let root = safe_absolute(root).map_err(failure)?;
    let parents = directory_handles(&root, false).map_err(failure)?;
    if !bundle_name(name, matches!(binding, Binding::Capture(_))) { return Err(invalid()); }
    let path = root.join(name);
    fs::create_dir(&path).map_err(failure)?;
    let directories = directory_handles(&path, true).map_err(failure)?;
    let directory = directories.last().ok_or_else(damaged)?;
    let mut bundle = Bundle { path: path.clone(), identity: file_identity(directory, true).map_err(failure)?,
        binding, files: Vec::new(), deleted_for: Vec::new() };
    let mut created = Vec::new();
    let result = (|| {
        for (name, bytes) in contents {
            if !safe_name(&name) || !allowed_file(&name, matches!(bundle.binding, Binding::Capture(_))) { return Err(invalid()); }
            let file = OpenOptions::new().read(true).write(true).access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                .create_new(true).share_mode(0).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path.join(&name)).map_err(failure)?;
            created.push(file);
            let file = created.last_mut().ok_or_else(damaged)?;
            write(file, &bytes).map_err(failure)?;
            bundle.files.push(OwnedFile { name, identity: file_identity(file, false).map_err(failure)?,
                fingerprint: Fingerprint::of(&bytes)? });
        }
        bundle.validate()
    })();
    if let Err(error) = result {
        for file in &created { let _ = mark_deleted(file); }
        drop(created);
        let _ = mark_deleted(directory);
        return Err(format!("{error}\n{}", path.display()));
    }
    drop(created);
    drop(directories);
    drop(parents);
    Ok(bundle)
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog { bundles: Vec<Bundle> }

fn catalog_path(history: &Path) -> PathBuf { history.with_file_name("feedback-local-files.dat") }

impl Catalog {
    fn prune_missing(&mut self) {
        self.bundles.retain(|bundle| {
            if !bundle.deleted_for.is_empty() { return true; }
            let Some(parent) = bundle.path.parent() else { return true; };
            let Ok(_parents) = directory_handles(parent, false) else { return true; };
            !fs::symlink_metadata(&bundle.path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        });
    }

    fn read(history: &Path) -> Result<Self, String> {
        let path = catalog_path(history);
        let _parents = directory_handles(path.parent().ok_or_else(damaged)?, false).map_err(failure)?;
        let mut file = match open_file(&path, false) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(failure(error)),
        };
        let bytes = read_bytes(&mut file, MAX_CATALOG).map_err(failure)?;
        let encrypted = bytes.strip_prefix(CATALOG_MAGIC).ok_or_else(damaged)?;
        let result: Self = serde_json::from_slice(&crypt(encrypted, false)?).map_err(|_| damaged())?;
        if result.bundles.len() > MAX_BUNDLES { return Err(damaged()); }
        let mut paths = BTreeSet::new();
        for bundle in &result.bundles {
            bundle.validate()?;
            if !paths.insert(bundle.path.to_string_lossy().to_lowercase()) { return Err(damaged()); }
        }
        Ok(result)
    }

    fn save(&self, history: &Path) -> Result<(), String> {
        let path = catalog_path(history);
        if self.bundles.is_empty() { return remove_history(&path); }
        let plain = serde_json::to_vec(self).map_err(|_| storage_error())?;
        if self.bundles.len() > MAX_BUNDLES || plain.len() > MAX_CATALOG - 65536 { return Err(storage_error()); }
        let _parents = directory_handles(path.parent().ok_or_else(damaged)?, false).map_err(failure)?;
        let temp = path.with_file_name(format!("feedback-files-{}.tmp", new_uuid()?));
        let encrypted = crypt(&plain, true)?;
        let result = (|| {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&temp).map_err(failure)?;
            file.write_all(CATALOG_MAGIC).and_then(|_| file.write_all(&encrypted))
                .and_then(|_| file.sync_all()).map_err(failure)?;
            drop(file);
            crate::paths::replace_file(&temp, &path).map_err(failure)
        })();
        if result.is_err() { let _ = remove_history(&temp); }
        result
    }

    fn add(&mut self, bundle: Bundle) -> Result<(), String> {
        bundle.validate()?;
        if self.bundles.iter().any(|existing| existing.path.to_string_lossy().eq_ignore_ascii_case(&bundle.path.to_string_lossy())) {
            return Err(damaged());
        }
        if self.bundles.len() >= MAX_BUNDLES { return Err(storage_error()); }
        self.bundles.push(bundle);
        Ok(())
    }
}

pub(crate) fn register(history: &Path, create: impl FnOnce() -> Result<Bundle, String>) -> Result<PathBuf, String> {
    let mut catalog = Catalog::read(history)?;
    catalog.prune_missing();
    if catalog.bundles.len() >= MAX_BUNDLES { return Err(storage_error()); }
    let bundle = create()?;
    let result = catalog.add(bundle.clone()).and_then(|()| catalog.save(history));
    if let Err(error) = result {
        let cleanup = bundle.remove();
        return Err(format!("{error}\n{}{}", bundle.path.display(), cleanup.err().map(|error| format!("\n{error}")).unwrap_or_default()));
    }
    Ok(bundle.path)
}

pub(crate) fn save_capture(create: impl FnOnce() -> Result<Bundle, String>) -> Result<PathBuf, String> {
    register_locked(&history_path(), create)
}

pub(super) fn register_locked(path: &Path, create: impl FnOnce() -> Result<Bundle, String>) -> Result<PathBuf, String> {
    with_storage_lock_at(path, true, |path| register(path, create))
}

pub(super) fn legacy_roots() -> Vec<PathBuf> {
    let cache = crate::paths::cache_dir();
    let config = crate::paths::config_dir();
    let mut roots = vec![cache.join("captures"), cache];
    if !roots.contains(&config) { roots.push(config); }
    roots
}

pub(super) fn delete_for(path: &Path, history: &History, ids: &[String], roots: &[PathBuf]) -> Result<(), String> {
    if !history.entries.iter().any(|entry| ids.contains(&entry.request_id)) { return Ok(()); }
    let mut catalog = Catalog::read(path)?;
    discover(&mut catalog, history, roots)?;
    catalog.save(path)?;
    let mut selected = Vec::new();
    for (index, bundle) in catalog.bundles.iter().enumerate() {
        let mut deleting = false;
        let mut keeping = false;
        for entry in &history.entries {
            if bundle.binding.matches(entry)? {
                if ids.contains(&entry.request_id) { deleting = true; } else { keeping = true; }
            }
        }
        if deleting && !keeping { selected.push(index); }
    }
    for index in selected {
        let bundle = &mut catalog.bundles[index];
        bundle.remove()?;
        bundle.deleted_for = ids.iter().filter(|id| history.entries.iter()
            .any(|entry| &entry.request_id == *id)).cloned().collect();
    }
    catalog.save(path)
}

pub(super) fn finish_delete(path: &Path, history: &History) -> Result<(), String> {
    let mut catalog = Catalog::read(path)?;
    let original = catalog.bundles.len();
    catalog.bundles.retain(|bundle| bundle.deleted_for.is_empty()
        || bundle.deleted_for.iter().any(|id| history.entries.iter().any(|entry| &entry.request_id == id)));
    catalog.prune_missing();
    if catalog.bundles.len() != original {
        catalog.save(path).map_err(|error| format!("{}\n{error}", message(
            "本地文件索引更新失败。", "Could not update the local file index.")))?;
    }
    Ok(())
}

fn discover(catalog: &mut Catalog, history: &History, roots: &[PathBuf]) -> Result<(), String> {
    let mut budget = MAX_HISTORY;
    for root in roots {
        let _parents = match directory_handles(root, false) {
            Ok(handles) => handles,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        };
        let Ok(items) = fs::read_dir(root) else { continue; };
        let mut candidates = 0;
        for item in items.take(4096).flatten() {
            if candidates >= 512 || budget == 0 || catalog.bundles.len() >= MAX_BUNDLES { break; }
            let path = item.path();
            if catalog.bundles.iter().any(|bundle| bundle.path.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy())) { continue; }
            let name = item.file_name();
            let Some(name) = name.to_str() else { continue; };
            let capture = bundle_name(name, true);
            if !capture && !bundle_name(name, false) { continue; }
            candidates += 1;
            if let Ok(Some(bundle)) = legacy_bundle(&path, capture, history, &mut budget) { catalog.add(bundle)?; }
        }
    }
    Ok(())
}

fn legacy_bundle(path: &Path, capture: bool, history: &History, budget: &mut usize) -> Result<Option<Bundle>, String> {
    let directories = match directory_handles(path, false) { Ok(handles) => handles, Err(_) => return Ok(None) };
    let mut contents = std::collections::BTreeMap::new();
    let mut files = Vec::new();
    for item in fs::read_dir(path).map_err(failure)?.take(9) {
        let item = item.map_err(failure)?;
        let Some(name) = item.file_name().to_str().map(str::to_owned) else { return Ok(None); };
        if !allowed_file(&name, capture) || files.len() >= 8 { return Ok(None); }
        let mut file = match open_file(&item.path(), false) { Ok(file) => file, Err(_) => return Ok(None) };
        let maximum = if name.starts_with("screenshot-") { MAX_IMAGE } else { MAX_CAPTURE_BYTES };
        if file.metadata().map_err(failure)?.len() > (*budget).min(maximum) as u64 {
            if *budget < maximum { *budget = 0; }
            return Ok(None);
        }
        let bytes = match read_bytes(&mut file, maximum) { Ok(bytes) => bytes, Err(_) => return Ok(None) };
        *budget = budget.checked_sub(bytes.len()).ok_or_else(storage_error)?;
        files.push(OwnedFile { name: name.clone(), identity: file_identity(&file, false).map_err(failure)?, fingerprint: Fingerprint::of(&bytes)? });
        contents.insert(name, bytes);
    }
    let binding = if capture {
        let Some(raw) = contents.get("raw.jsonl") else { return Ok(None); };
        let Some(summary) = contents.get("summary.txt") else { return Ok(None); };
        if super::capture(raw).is_err() || !crate::capture::matches_saved_summary(raw, summary) { return Ok(None); }
        Binding::capture(raw)?
    } else {
        let Some(binding) = export::legacy_binding(&contents, history)? else { return Ok(None); };
        binding
    };
    let mut matches = false;
    for entry in &history.entries { matches |= binding.matches(entry)?; }
    if !matches { return Ok(None); }
    Ok(Some(Bundle { path: safe_absolute(path).map_err(failure)?,
        identity: file_identity(directories.last().ok_or_else(damaged)?, true).map_err(failure)?, binding, files, deleted_for: Vec::new() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{draft, raw_capture, screenshot, Scratch};

    fn history_path_at(scratch: &Scratch) -> PathBuf { scratch.0.join("feedback-history.dat") }

    fn pending(history: &mut History, draft: Draft) -> StoredEntry {
        prepare_entry(history, draft, "1.4.12").unwrap()
    }

    fn capture_bundle(root: &Path, raw: &[u8]) -> Bundle {
        let metadata: serde_json::Value = serde_json::from_slice(raw.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
        let vendor = metadata["vendor_id"].as_u64().unwrap();
        let product = metadata["product_id"].as_u64().unwrap();
        write_bundle(root, &format!("capture-{}-{}-1", now(), std::process::id()), Binding::capture(raw).unwrap(),
            vec![("raw.jsonl".into(), raw.to_vec()), ("summary.txt".into(),
                format!("Feedback capture: {vendor:04X}:{product:04X}\nFile: raw.jsonl\n").into_bytes())],
            |file, bytes| file.write_all(bytes).and_then(|_| file.sync_all())).unwrap()
    }

    #[test]
    fn completed_shared_exports_survive_until_the_last_owner_without_deleting_originals() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let mut initial = draft();
        initial.diagnostics = Some("Captured at (Unix ms): 100\nBattery: 50%".into());
        initial.images.push(screenshot());
        let raw = raw_capture("AABB");
        initial.capture = Some(capture(&raw).unwrap());
        let original = scratch.0.join("original-upload.png");
        fs::write(&original, STANDARD.decode(&initial.images[0].data_base64).unwrap()).unwrap();
        let exported = register(&path, || export::save(&initial, "1.4.12", None, None, &scratch.0)).unwrap();
        let captured = register(&path, || Ok(capture_bundle(&scratch.0, &raw))).unwrap();
        let unknown = exported.join("user-notes.txt");
        fs::write(&unknown, b"keep").unwrap();
        let mut submitted = initial.clone();
        submitted.diagnostics = Some("Captured at (Unix ms): 200\nBattery: 49%".into());
        let mut history = History::default();
        let first = pending(&mut history, submitted.clone());
        complete_entry(&mut history, &first, CreateReceipt { number: 42,
            url: format!("{REPOSITORY}/issues/42"), reply_access: true }).unwrap();
        assert!(history.entries[0].draft.images.is_empty());
        assert!(history.entries[0].draft.capture.is_none());
        let second = pending(&mut history, submitted);
        save_history(&path, &history).unwrap();
        delete_local_at(&path, &[first.request_id], &[]).unwrap();
        assert!(exported.join("raw.jsonl").exists());
        assert!(captured.exists());
        assert_eq!(read_history(&path).unwrap().entries[0].request_id, second.request_id);
        delete_local_at(&path, &[second.request_id.clone(), second.request_id], &[]).unwrap();
        assert!(!path.exists());
        assert!(!catalog_path(&path).exists());
        assert!(!exported.join("feedback.json").exists());
        assert!(!captured.exists());
        assert_eq!(fs::read(&unknown).unwrap(), b"keep");
        assert!(original.exists());
        assert!(scratch.0.is_dir());
    }

    #[test]
    fn request_exports_are_owned_by_the_exact_feedback() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let mut history = History::default();
        let first = pending(&mut history, draft());
        complete_entry(&mut history, &first, CreateReceipt { number: 42,
            url: format!("{REPOSITORY}/issues/42"), reply_access: true }).unwrap();
        let second = pending(&mut history, draft());
        save_history(&path, &history).unwrap();
        let folder = register(&path, || export::save(&first.draft, &first.app_version,
            Some(&first.public()), None, &scratch.0)).unwrap();
        delete_local_at(&path, &[first.request_id], &[]).unwrap();
        assert!(!folder.exists());
        assert!(!catalog_path(&path).exists());
        assert_eq!(read_history(&path).unwrap().entries[0].request_id, second.request_id);
    }

    #[test]
    fn deletion_retains_the_catalog_until_history_commits_and_can_retry_a_locked_history() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let mut history = History::default();
        let entry = pending(&mut history, draft());
        save_history(&path, &history).unwrap();
        let folder = register(&path, || export::save(&entry.draft, &entry.app_version,
            Some(&entry.public()), None, &scratch.0)).unwrap();
        let ids = [entry.request_id];
        delete_for(&path, &history, &ids, &[]).unwrap();
        assert!(!folder.exists());
        assert_eq!(Catalog::read(&path).unwrap().bundles[0].deleted_for, ids);
        finish_delete(&path, &history).unwrap();
        assert_eq!(Catalog::read(&path).unwrap().bundles.len(), 1);
        let locked = open_file(&path, false).unwrap();
        assert!(delete_local_at(&path, &ids, &[]).is_err());
        assert_eq!(read_history(&path).unwrap().entries.len(), 1);
        assert!(catalog_path(&path).exists());
        drop(locked);
        delete_local_at(&path, &ids, &[]).unwrap();
        assert!(!path.exists());
        assert!(!catalog_path(&path).exists());
    }

    #[test]
    fn catalog_pruning_can_be_retried_after_the_history_was_already_removed() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let mut history = History::default();
        let entry = pending(&mut history, draft());
        save_history(&path, &history).unwrap();
        register(&path, || export::save(&entry.draft, &entry.app_version, Some(&entry.public()), None, &scratch.0)).unwrap();
        let ids = [entry.request_id];
        delete_for(&path, &history, &ids, &[]).unwrap();
        remove_entries(&mut history, &ids).unwrap();
        save_history(&path, &history).unwrap();
        let locked = open_file(&catalog_path(&path), false).unwrap();
        assert!(finish_delete(&path, &history).is_err());
        assert!(!path.exists());
        drop(locked);
        delete_local_at(&path, &ids, &[]).unwrap();
        assert!(!catalog_path(&path).exists());
    }

    #[test]
    fn modified_files_block_deletion_and_preserve_the_retry_association() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let mut history = History::default();
        let entry = pending(&mut history, draft());
        save_history(&path, &history).unwrap();
        let folder = register(&path, || export::save(&entry.draft, &entry.app_version,
            Some(&entry.public()), None, &scratch.0)).unwrap();
        let report = folder.join("report.md");
        let bytes = fs::read(&report).unwrap();
        fs::write(&report, b"user changed this file").unwrap();
        let ids = [entry.request_id];
        assert!(delete_local_at(&path, &ids, &[]).is_err());
        assert!(folder.join("feedback.json").exists());
        assert_eq!(fs::read(&report).unwrap(), b"user changed this file");
        assert_eq!(read_history(&path).unwrap().entries.len(), 1);
        fs::write(&report, bytes).unwrap();
        delete_local_at(&path, &ids, &[]).unwrap();
        assert!(!folder.exists());
    }

    #[test]
    fn bundle_paths_duplicate_registration_and_hardlinks_are_rejected() {
        let scratch = Scratch::new();
        let bundle = export::save(&draft(), "1.4.12", None, None, &scratch.0).unwrap();
        let mut catalog = Catalog::default();
        catalog.add(bundle.clone()).unwrap();
        assert!(catalog.add(bundle.clone()).is_err());
        let mut tampered = bundle.clone();
        tampered.path = scratch.0.clone();
        assert!(tampered.remove().is_err());
        tampered = bundle.clone();
        tampered.files[0].name = "../original.png".into();
        assert!(tampered.remove().is_err());
        tampered = bundle.clone();
        tampered.path = bundle.path.join("..").join(bundle.path.file_name().unwrap());
        assert!(tampered.remove().is_err());
        let original = scratch.0.join("original.txt");
        let report = bundle.path.join("report.md");
        let bytes = fs::read(&report).unwrap();
        fs::write(&original, &bytes).unwrap();
        fs::remove_file(&report).unwrap();
        fs::hard_link(&original, &report).unwrap();
        assert!(bundle.remove().is_err());
        assert_eq!(fs::read(&original).unwrap(), bytes);
        assert!(report.exists());
    }

    #[test]
    fn replacing_a_bundle_directory_with_a_reparse_point_never_deletes_the_target() {
        let scratch = Scratch::new();
        let bundle = export::save(&draft(), "1.4.12", None, None, &scratch.0).unwrap();
        let moved = scratch.0.join("moved");
        fs::rename(&bundle.path, &moved).unwrap();
        match std::os::windows::fs::symlink_dir(&moved, &bundle.path) {
            Ok(()) => (),
            Err(error) if matches!(error.raw_os_error(), Some(5 | 1314)) => return,
            Err(error) => panic!("{error}"),
        }
        assert!(bundle.remove().is_err());
        assert!(moved.join("feedback.json").exists());
        fs::remove_dir(&bundle.path).unwrap();
    }

    #[test]
    fn legacy_discovery_requires_content_and_never_guesses_for_completed_records() {
        let scratch = Scratch::new();
        let root = scratch.0.join("controlled");
        fs::create_dir(&root).unwrap();
        let path = history_path_at(&scratch);
        let mut initial = draft();
        let raw = raw_capture("ABCD");
        initial.capture = Some(capture(&raw).unwrap());
        let mut history = History::default();
        let first = pending(&mut history, initial.clone());
        let legacy_capture = capture_bundle(&root, &raw).path;
        let legacy_export = export::save(&initial, "1.4.12", None, None, &root).unwrap().path;
        let unrelated = export::save(&initial, "1.4.12", None, None, &scratch.0).unwrap().path;
        history.entries[0].local_identity = None;
        save_history(&path, &history).unwrap();
        delete_local_at(&path, &[first.request_id], std::slice::from_ref(&root)).unwrap();
        assert!(!legacy_capture.exists());
        assert!(!legacy_export.exists());
        assert!(unrelated.exists());
        let mut history = History::default();
        let completed = pending(&mut history, initial.clone());
        complete_entry(&mut history, &completed, CreateReceipt { number: 43,
            url: format!("{REPOSITORY}/issues/43"), reply_access: true }).unwrap();
        history.entries[0].local_identity = None;
        let unproven = export::save(&initial, "1.4.12", None, None, &root).unwrap().path;
        let proven = export::save(&initial, "1.4.12", Some(&completed.public()), None, &root).unwrap().path;
        save_history(&path, &history).unwrap();
        delete_local_at(&path, &[completed.request_id], std::slice::from_ref(&root)).unwrap();
        assert!(unproven.exists());
        assert!(!proven.exists());
    }

    #[test]
    fn legacy_scan_limits_do_not_block_registered_cleanup() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let root = scratch.0.join("controlled");
        fs::create_dir(&root).unwrap();
        for index in 0..520 { fs::write(root.join(format!("capture-{index}-1-1")), b"unrelated").unwrap(); }
        let mut history = History::default();
        let entry = pending(&mut history, draft());
        save_history(&path, &history).unwrap();
        let folder = register(&path, || export::save(&entry.draft, "1.4.12", None, None, &scratch.0)).unwrap();
        delete_local_at(&path, &[entry.request_id], &[root.clone()]).unwrap();
        assert!(!folder.exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 520);
    }

    #[test]
    fn deletion_waits_for_the_writer_and_preserves_its_new_record() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let mut history = History::default();
        let first = pending(&mut history, draft());
        save_history(&path, &history).unwrap();
        let (ready_sender, ready_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || with_history_at(&writer_path, true, |_, history| {
            let mut next = draft();
            next.description = "A separately submitted new feedback record".into();
            let second = pending(history, next);
            ready_sender.send(second.request_id.clone()).unwrap();
            release_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(second.request_id)
        }));
        let second_id = ready_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let delete_path = path.clone();
        let deleting = std::thread::spawn(move || delete_local_at(&delete_path, &[first.request_id], &[]));
        release_sender.send(()).unwrap();
        assert_eq!(writer.join().unwrap().unwrap(), second_id);
        deleting.join().unwrap().unwrap();
        let remaining = read_history(&path).unwrap();
        assert_eq!(remaining.entries.len(), 1);
        assert_eq!(remaining.entries[0].request_id, second_id);
    }

    #[test]
    fn offline_draft_and_capture_exports_do_not_read_or_rewrite_damaged_history() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let damaged = b"damaged encrypted history";
        fs::write(&path, damaged).unwrap();
        let exported = register_locked(&path, || export::save(&draft(), "1.4.12", None, None, &scratch.0)).unwrap();
        let raw = raw_capture("CAFE");
        let captured = register_locked(&path, || Ok(capture_bundle(&scratch.0, &raw))).unwrap();
        assert!(exported.join("feedback.json").exists());
        assert_eq!(fs::read(captured.join("raw.jsonl")).unwrap(), raw);
        assert_eq!(fs::read(&path).unwrap(), damaged);
        assert_eq!(Catalog::read(&path).unwrap().bundles.len(), 2);
    }

    #[test]
    fn manually_removed_exports_release_catalog_capacity_without_touching_other_files() {
        let scratch = Scratch::new();
        let path = history_path_at(&scratch);
        let first = register(&path, || export::save(&draft(), "1.4.12", None, None, &scratch.0)).unwrap();
        let bundle = Catalog::read(&path).unwrap().bundles.remove(0);
        bundle.remove().unwrap();
        assert!(!first.exists());
        let mut catalog = Catalog::default();
        for _index in 0..MAX_BUNDLES {
            let mut missing = bundle.clone();
            missing.path = scratch.0.join(format!("X-Battery-feedback-1-{}", new_uuid().unwrap()));
            catalog.add(missing).unwrap();
        }
        catalog.save(&path).unwrap();
        let exported = register(&path, || export::save(&draft(), "1.4.12", None, None, &scratch.0)).unwrap();
        let catalog = Catalog::read(&path).unwrap();
        assert_eq!(catalog.bundles.len(), 1);
        assert_eq!(catalog.bundles[0].path, exported);
        assert!(exported.join("feedback.json").exists());
    }

    #[test]
    fn path_validation_accepts_network_shares_and_rejects_device_namespaces() {
        for path in [r"\\server\share\feedback", r"\\?\UNC\server\share\feedback", r"C:\feedback"] {
            assert!(safe_absolute(Path::new(path)).is_ok(), "{path}");
        }
        for path in [r"\\.\PhysicalDrive0", r"\\?\GLOBALROOT\Device\HarddiskVolume1", r"\\server\share\..\private"] {
            assert!(safe_absolute(Path::new(path)).is_err(), "{path}");
        }
    }
}
