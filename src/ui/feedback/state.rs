use crate::feedback::{client, Candidate};

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum View {
    Form,
    Diagnostics,
    Capture,
    History,
    Thread,
    Success,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Task {
    History,
    Submit,
    Thread,
    Reply,
    Draft,
    Images,
    Fallback,
    SoftwareDiagnostics,
    CaptureExport,
    HistoryDelete,
    FeedbackExport,
    RetryEntry,
}

impl Task {
    pub(super) fn locks_editors(self) -> bool {
        !matches!(self, Self::History | Self::Thread | Self::SoftwareDiagnostics)
    }
}

pub(super) enum DiagnosticReport {
    Summary(String),
    Complete(String),
}

pub(super) struct Model {
    pub(super) view: View,
    pub(super) devices: Vec<Candidate>,
    pub(super) latest_devices: Vec<Candidate>,
    pub(super) initialized: bool,
    pub(super) software: bool,
    pub(super) latest_software: bool,
    pub(super) menu: bool,
    pub(super) device_page: usize,
    pub(super) selected: Option<usize>,
    pub(super) unknown_devices: Vec<crate::feedback::inventory::UnknownDevice>,
    pub(super) selected_unknown: Option<usize>,
    pub(super) connection: usize,
    pub(super) category: usize,
    pub(super) attach_diagnostics: bool,
    pub(super) images: Vec<client::Image>,
    pub(super) capture: Option<client::Capture>,
    pub(super) attach_capture: bool,
    pub(super) entries: Vec<client::Entry>,
    pub(super) history_page: usize,
    pub(super) current: Option<client::Entry>,
    pub(super) thread: Option<client::Thread>,
    pub(super) draft_retry: Option<String>,
    pub(super) error: String,
    pub(super) task: Option<Task>,
    pub(super) submitted: bool,
    pub(super) diagnostics_override: Option<DiagnosticReport>,
    pub(super) inventory_report: Option<String>,
    pub(super) inventory_generation: u64,
    pub(super) restored_draft: Option<client::Draft>,
    pub(super) submitted_draft: Option<client::Draft>,
    pub(super) restored_device: Option<client::Device>,
    pub(super) reply_drafts: std::collections::BTreeMap<u64, String>,
    generation: u64,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            view: View::Form,
            devices: Vec::new(),
            latest_devices: Vec::new(),
            initialized: false,
            software: false,
            latest_software: false,
            menu: false,
            device_page: 0,
            selected: None,
            unknown_devices: Vec::new(),
            selected_unknown: None,
            connection: 0,
            category: 0,
            attach_diagnostics: true,
            images: Vec::new(),
            capture: None,
            attach_capture: true,
            entries: Vec::new(),
            history_page: 0,
            current: None,
            thread: None,
            draft_retry: None,
            error: String::new(),
            task: None,
            submitted: false,
            diagnostics_override: None,
            inventory_report: None,
            inventory_generation: 0,
            restored_draft: None,
            submitted_draft: None,
            restored_device: None,
            reply_drafts: Default::default(),
            generation: 0,
        }
    }
}

impl Model {
    pub(super) fn has_complete_diagnostics(&self) -> bool {
        matches!(self.diagnostics_override, Some(DiagnosticReport::Complete(_)))
    }

    pub(super) fn busy(&self) -> bool {
        self.task.is_some()
    }

    pub(super) fn begin_task(&mut self, task: Task) -> Option<u64> {
        if self.busy() {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.task = Some(task);
        self.error.clear();
        Some(self.generation)
    }

    pub(super) fn finish_task(&mut self, generation: u64) -> bool {
        if self.generation != generation || self.task.is_none() {
            return false;
        }
        self.task = None;
        true
    }

    pub(super) fn detach_task(&mut self) {
        if matches!(self.task, Some(Task::CaptureExport | Task::HistoryDelete | Task::FeedbackExport)) { return; }
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
    }

    pub(super) fn new_form(&mut self) {
        if matches!(self.task, Some(Task::CaptureExport | Task::HistoryDelete | Task::FeedbackExport)) { return; }
        let mode_changed = self.software != self.latest_software;
        self.detach_task();
        self.view = View::Form;
        self.submitted = false;
        self.draft_retry = None;
        self.restored_draft = None;
        self.submitted_draft = None;
        self.restored_device = None;
        self.diagnostics_override = None;
        self.inventory_report = None;
        self.unknown_devices.clear();
        self.selected_unknown = None;
        self.inventory_generation = self.inventory_generation.wrapping_add(1);
        self.error.clear();
        self.images.clear();
        self.capture = None;
        self.attach_capture = true;
        self.current = None;
        self.thread = None;
        self.menu = false;
        self.device_page = 0;
        self.software = self.latest_software;
        if self.software || mode_changed {
            self.category = if self.software { 3 } else { 0 };
            self.attach_diagnostics = true;
        }
        self.devices = self.latest_devices.clone();
        self.selected = (!self.devices.is_empty()).then_some(0);
        self.connection = self.selected
            .map(|index| super::connection_index(&self.devices[index].device.connection))
            .unwrap_or(3);
        self.initialized = true;
    }

    pub(super) fn reopen(&mut self, devices: Vec<Candidate>, history: bool) -> bool {
        self.reopen_mode(devices, history, false)
    }

    pub(super) fn reopen_software(&mut self) -> bool {
        self.reopen_mode(Vec::new(), false, true)
    }

    fn reopen_mode(&mut self, devices: Vec<Candidate>, history: bool, software: bool) -> bool {
        if matches!(self.task, Some(Task::CaptureExport | Task::HistoryDelete | Task::FeedbackExport)) { return false; }
        self.refresh_devices(devices);
        if !history {
            self.latest_software = software;
        }
        let reset = !self.initialized || (!history && (self.submitted || self.software != software));
        if reset {
            self.new_form();
        }
        if history {
            self.detach_task();
            self.view = View::History;
        } else if !self.busy() || self.task.is_some_and(|task| !task.locks_editors()) {
            self.detach_task();
            self.view = View::Form;
        }
        reset
    }

    pub(super) fn edit_draft(&mut self) -> bool {
        self.draft_retry = None;
        self.restored_draft = None;
        self.submitted_draft = None;
        let had_error = !self.error.is_empty();
        self.error.clear();
        had_error
    }

    pub(super) fn refresh_devices(&mut self, devices: Vec<Candidate>) {
        self.latest_devices = devices;
        if self.software || self.submitted || self.draft_retry.is_some()
            || self.restored_draft.is_some() || self.submitted_draft.is_some()
            || self.task.is_some_and(Task::locks_editors) { return; }
        let selected = self.selected.and_then(|index| self.devices.get(index)).cloned();
        self.devices = self.latest_devices.clone();
        if let Some(selected) = selected {
            self.selected = self.devices.iter().position(|device| device.same_source(&selected));
            if self.selected.is_none() {
                self.selected = Some(self.devices.len());
                self.devices.push(selected.unavailable());
            }
        }
        self.device_page = self.device_page.min(self.devices.len().saturating_sub(1) / super::DEVICE_PAGE_SIZE);
    }

    pub(super) fn record_entry(&mut self, entry: client::Entry) {
        self.entries.retain(|stored| stored.request_id != entry.request_id);
        self.entries.insert(0, entry.clone());
        self.entries.truncate(128);
        self.current = Some(entry);
    }

    pub(super) fn remove_local_entries(&mut self, request_ids: &[String]) -> bool {
        for entry in &self.entries {
            if request_ids.contains(&entry.request_id) {
                if let Some(number) = entry.number { self.reply_drafts.remove(&number); }
            }
        }
        self.entries.retain(|entry| !request_ids.contains(&entry.request_id));
        let reset = self.current.as_ref().is_some_and(|entry| request_ids.contains(&entry.request_id))
            || self.draft_retry.as_ref().is_some_and(|id| request_ids.contains(id));
        if reset { self.new_form(); }
        self.view = View::History;
        self.history_page = self.history_page.min(self.entries.len().saturating_sub(1) / super::HISTORY_PAGE_SIZE);
        reset
    }

    pub(super) fn confirm_reply(&mut self, number: u64, sent: &str) {
        if self.reply_drafts.get(&number).is_some_and(|draft| draft == sent) {
            self.reply_drafts.remove(&number);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_feedback_requires_a_new_inventory_without_guessing_the_connection() {
        let mut model = Model::default();
        model.reopen(Vec::new(), false);
        let generation = model.inventory_generation;
        model.inventory_report = Some("previous interfaces".into());
        assert_eq!(model.connection, 3);
        model.new_form();
        assert_ne!(model.inventory_generation, generation);
        assert!(model.inventory_report.is_none());
        assert!(!model.busy());
    }

    fn entry(number: u64) -> client::Entry {
        client::Entry {
            request_id: format!("00000000-0000-4000-8000-{number:012}"),
            created_at: number,
            updated_at: number,
            category: "battery".into(),
            device: client::Device {
                name: "Example mouse".into(),
                family: "mouse".into(),
                connection: "USB".into(),
                brand: None,
                model: None,
            },
            description: "Battery report".into(),
            status: client::Status::Completed,
            number: Some(number),
            url: Some(format!("https://github.com/youmikk/X-Battery/issues/{number}")),
        }
    }

    #[test]
    fn reopening_after_success_starts_a_new_form_without_erasing_history() {
        let mut model = Model::default();
        model.record_entry(entry(1));
        model.submitted = true;
        model.view = View::Success;
        model.initialized = true;
        model.draft_retry = Some("old-request".into());
        model.error = "old-error".into();
        assert!(model.reopen(Vec::new(), false));
        assert_eq!(model.view, View::Form);
        assert!(!model.submitted);
        assert!(model.draft_retry.is_none());
        assert!(model.current.is_none());
        assert!(model.error.is_empty());
        assert_eq!(model.entries.len(), 1);
    }

    #[test]
    fn reopening_an_unsubmitted_form_preserves_its_retry_identity() {
        let mut model = Model::default();
        model.initialized = true;
        model.draft_retry = Some("pending-request".into());
        assert!(!model.reopen(Vec::new(), false));
        assert_eq!(model.draft_retry.as_deref(), Some("pending-request"));
    }

    #[test]
    fn opening_history_does_not_reset_a_completed_form_or_its_receipt() {
        let mut model = Model::default();
        model.initialized = true;
        model.submitted = true;
        model.current = Some(entry(1));
        assert!(!model.reopen(Vec::new(), true));
        assert_eq!(model.view, View::History);
        assert!(model.submitted);
        assert_eq!(model.current.as_ref().unwrap().number, Some(1));
    }

    #[test]
    fn new_feedback_detaches_slow_reply_loading_and_rejects_its_late_result() {
        let mut model = Model::default();
        let old_generation = model.begin_task(Task::Thread).unwrap();
        model.new_form();
        let new_generation = model.begin_task(Task::Submit).unwrap();
        assert!(!model.finish_task(old_generation));
        assert_eq!(model.task, Some(Task::Submit));
        assert!(model.finish_task(new_generation));
        assert!(!model.busy());
    }

    #[test]
    fn reopening_during_submission_preserves_the_inflight_form() {
        let mut model = Model::default();
        model.initialized = true;
        let generation = model.begin_task(Task::Submit).unwrap();
        assert!(!model.reopen(Vec::new(), false));
        assert_eq!(model.task, Some(Task::Submit));
        assert!(model.finish_task(generation));
    }

    #[test]
    fn history_loading_can_be_left_without_waiting_for_the_worker() {
        let mut model = Model::default();
        model.initialized = true;
        model.view = View::History;
        let generation = model.begin_task(Task::History).unwrap();
        assert!(!model.reopen(Vec::new(), false));
        assert_eq!(model.view, View::Form);
        assert!(!model.finish_task(generation));
    }

    #[test]
    fn two_completed_feedback_sessions_keep_two_independent_receipts() {
        let mut model = Model::default();
        model.record_entry(entry(1));
        model.submitted = true;
        model.new_form();
        model.record_entry(entry(2));
        assert_eq!(model.entries.len(), 2);
        assert_eq!(model.entries[0].number, Some(2));
        assert_eq!(model.entries[1].number, Some(1));
    }

    #[test]
    fn retry_receipts_update_existing_history_instead_of_duplicating_it() {
        let mut model = Model::default();
        model.record_entry(entry(1));
        model.record_entry(entry(1));
        assert_eq!(model.entries.len(), 1);
    }

    #[test]
    fn deleting_the_current_record_clears_its_draft_and_ignores_late_results() {
        let mut model = Model::default();
        let removed = entry(1);
        model.record_entry(entry(2));
        model.record_entry(removed.clone());
        model.reply_drafts.insert(1, "unsent reply".into());
        model.reply_drafts.insert(2, "keep reply".into());
        model.draft_retry = Some(removed.request_id.clone());
        let generation = model.begin_task(Task::Submit).unwrap();
        assert!(model.remove_local_entries(&[removed.request_id]));
        assert!(!model.finish_task(generation));
        assert!(model.current.is_none());
        assert!(model.draft_retry.is_none());
        assert!(!model.reply_drafts.contains_key(&1));
        assert!(model.reply_drafts.contains_key(&2));
        assert_eq!(model.entries.len(), 1);
        assert_eq!(model.view, View::History);
    }

    #[test]
    fn a_busy_session_cannot_start_duplicate_tasks() {
        let mut model = Model::default();
        let generation = model.begin_task(Task::Submit).unwrap();
        assert!(model.begin_task(Task::Submit).is_none());
        assert!(model.finish_task(generation));
        assert!(!model.finish_task(generation));
    }

    #[test]
    fn typing_clears_stale_errors_and_retry_snapshots() {
        let mut model = Model::default();
        model.draft_retry = Some("pending-request".into());
        model.error = "validation failed".into();
        assert!(model.edit_draft());
        assert!(model.draft_retry.is_none());
        assert!(!model.edit_draft());
    }

    #[test]
    fn local_mutations_cannot_detach_or_reset_until_their_results_arrive() {
        for task in [Task::CaptureExport, Task::HistoryDelete, Task::FeedbackExport] {
            let mut model = Model::default();
            model.initialized = true;
            model.view = View::History;
            let generation = model.begin_task(task).unwrap();
            model.detach_task();
            model.new_form();
            assert!(!model.reopen(Vec::new(), false));
            assert!(!model.reopen_software());
            assert_eq!(model.view, View::History);
            assert_eq!(model.task, Some(task));
            assert!(model.finish_task(generation));
        }
    }

    #[test]
    fn reading_replies_does_not_lock_the_reply_editor() {
        assert!(!Task::Thread.locks_editors());
        assert!(!Task::History.locks_editors());
        assert!(!Task::SoftwareDiagnostics.locks_editors());
        assert!(Task::Submit.locks_editors());
        assert!(Task::Reply.locks_editors());
    }

    #[test]
    fn a_late_reply_receipt_clears_only_the_submitted_draft() {
        let mut model = Model::default();
        model.reply_drafts.insert(1, "First follow-up".into());
        model.reply_drafts.insert(2, "Another follow-up".into());
        let old_generation = model.begin_task(Task::Reply).unwrap();
        model.new_form();
        let new_generation = model.begin_task(Task::Submit).unwrap();
        model.confirm_reply(1, "First follow-up");
        assert!(!model.reply_drafts.contains_key(&1));
        assert_eq!(model.reply_drafts.get(&2).map(String::as_str), Some("Another follow-up"));
        assert_eq!(model.view, View::Form);
        assert!(!model.finish_task(old_generation));
        assert_eq!(model.task, Some(Task::Submit));
        assert!(model.finish_task(new_generation));
    }

    #[test]
    fn a_late_reply_receipt_preserves_a_changed_draft() {
        let mut model = Model::default();
        model.reply_drafts.insert(1, "Updated follow-up".into());
        model.confirm_reply(1, "First follow-up");
        assert_eq!(model.reply_drafts.get(&1).map(String::as_str), Some("Updated follow-up"));
    }

    #[test]
    fn software_feedback_does_not_require_a_device_and_new_forms_keep_their_mode() {
        let mut model = Model::default();
        assert!(model.reopen_software());
        assert!(model.software);
        assert_eq!(model.category, 3);
        assert!(model.selected.is_none());
        model.record_entry(entry(1));
        model.submitted = true;
        model.new_form();
        assert!(model.software);
        assert!(model.attach_diagnostics);
        assert_eq!(model.entries.len(), 1);
        assert!(model.reopen(Vec::new(), false));
        assert!(!model.software);
        assert_eq!(model.category, 0);
    }

    #[test]
    fn software_diagnostic_collection_can_be_detached_without_locking_the_description() {
        let mut model = Model::default();
        model.reopen_software();
        let generation = model.begin_task(Task::SoftwareDiagnostics).unwrap();
        assert!(!model.task.unwrap().locks_editors());
        model.new_form();
        assert!(!model.finish_task(generation));
        assert!(model.software);
    }

    #[test]
    fn opening_history_keeps_the_software_form_mode_and_pending_receipt() {
        let mut model = Model::default();
        model.reopen_software();
        model.draft_retry = Some("pending-software-request".into());
        assert!(!model.reopen(Vec::new(), true));
        assert!(model.software);
        assert!(model.latest_software);
        assert_eq!(model.draft_retry.as_deref(), Some("pending-software-request"));
    }
}
