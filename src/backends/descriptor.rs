//! Inspect report capabilities without opening an input or output stream.
use crate::{hid::HidInterface, win32::*};
use std::collections::BTreeMap;
use winapi::shared::hidpi::{HIDP_BUTTON_CAPS, HIDP_VALUE_CAPS};

pub(super) struct Descriptor {
    api: HidApi,
    handle: Handle,
    preparsed: *mut core::ffi::c_void,
    caps: HidpCaps,
}

impl Descriptor {
    pub fn open(api: &HidApi, interface: &HidInterface) -> Option<Self> {
        let path = to_wide(&interface.path);
        let handle = unsafe { CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null_mut(), OPEN_EXISTING, 0, core::ptr::null_mut()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE { return None; }
        let mut preparsed = core::ptr::null_mut();
        if unsafe { (api.get_preparsed_data)(handle, &mut preparsed) } == 0 {
            unsafe { CloseHandle(handle); }
            return None;
        }
        let mut descriptor = Self { api: *api, handle, preparsed, caps: HidpCaps::default() };
        if unsafe { (api.get_caps)(descriptor.preparsed, &mut descriptor.caps) } < 0 { return None; }
        Some(descriptor)
    }

    fn report_ids(&self, kind: u32) -> Option<Vec<u8>> {
        let (button_count, value_count) = match kind {
            0 => (self.caps.number_input_button_caps, self.caps.number_input_value_caps),
            1 => (self.caps.number_output_button_caps, self.caps.number_output_value_caps),
            2 => (self.caps.number_feature_button_caps, self.caps.number_feature_value_caps),
            _ => return None,
        };
        collect_ids(button_count, value_count, |buttons, count| {
            let mut returned = count;
            if buttons {
                let mut caps: Vec<HIDP_BUTTON_CAPS> = (0..count).map(|_| unsafe { core::mem::zeroed() }).collect();
                if unsafe { (self.api.get_button_caps)(kind, caps.as_mut_ptr(), &mut returned, self.preparsed) } < 0
                    || returned != count { return None; }
                Some(caps.iter().map(|cap| cap.ReportID).collect())
            } else {
                let mut caps: Vec<HIDP_VALUE_CAPS> = (0..count).map(|_| unsafe { core::mem::zeroed() }).collect();
                if unsafe { (self.api.get_value_caps)(kind, caps.as_mut_ptr(), &mut returned, self.preparsed) } < 0
                    || returned != count { return None; }
                Some(caps.iter().map(|cap| cap.ReportID).collect())
            }
        })
    }

    pub fn only_input_output_report(&self, interface: &HidInterface, id: u8, length: u16) -> bool {
        self.only_input_output_report_lengths(interface, id, length, length)
    }

    pub fn only_input_output_report_lengths(&self, interface: &HidInterface, id: u8,
        input: u16, output: u16) -> bool {
        self.only_reports(interface, id, input, id, output)
    }

    pub fn only_reports(&self, interface: &HidInterface, input_id: u8, input: u16,
        output_id: u8, output: u16) -> bool {
        self.caps.usage_page == interface.usage_page && self.caps.usage == interface.usage
            && input > 0 && output > 0
            && self.caps.input_report_byte_length == input && self.caps.output_report_byte_length == output
            && [(0, input_id), (1, output_id)].into_iter().all(|(kind, id)| self.report_ids(kind)
                .is_some_and(|ids| !ids.is_empty() && ids.iter().all(|actual| *actual == id)))
    }

    pub fn feature_reports(&self, interface: &HidInterface, length: u16, required: &[u8], allowed: &[u8]) -> bool {
        self.caps.usage_page == interface.usage_page && self.caps.usage == interface.usage
            && length > 0 && self.caps.feature_report_byte_length == length
            && self.report_ids(2).is_some_and(|ids| !ids.is_empty()
                && required.iter().all(|id| ids.contains(id)) && ids.iter().all(|id| allowed.contains(id)))
    }

    /// Prove one simple byte-array feature report without assuming that the
    /// collection maximum belongs to the report being transferred.
    pub fn feature_byte_report(&self, interface: &HidInterface, id: u8, length: u16) -> bool {
        if self.caps.usage_page != interface.usage_page
            || self.caps.usage != interface.usage
            || self.caps.number_feature_button_caps != 0
            || self.caps.feature_report_byte_length < length
        {
            return false;
        }
        let count = self.caps.number_feature_value_caps;
        if !(1..=32).contains(&count) {
            return false;
        }
        let mut values: Vec<HIDP_VALUE_CAPS> =
            (0..count).map(|_| unsafe { core::mem::zeroed() }).collect();
        let mut returned = count;
        if unsafe {
            (self.api.get_value_caps)(2, values.as_mut_ptr(), &mut returned, self.preparsed)
        } < 0
            || returned != count
        {
            return false;
        }
        feature_byte_report_lengths(&values, self.caps.feature_report_byte_length)
            .is_some_and(|sizes| sizes.get(&id) == Some(&length))
    }

    /// Prove that every advertised input report ID is in a small protocol
    /// allow-list. The caller must separately validate the collection usage.
    pub fn input_report_ids(&self, interface: &HidInterface, length: u16, allowed: &[u8]) -> bool {
        self.caps.usage_page == interface.usage_page && self.caps.usage == interface.usage
            && length > 0 && self.caps.input_report_byte_length == length
            && self.report_ids(0).is_some_and(|ids| !ids.is_empty()
                && ids.iter().all(|id| allowed.contains(id)))
    }

    /// Return the sole input and output report IDs after proving both Windows
    /// capacities. Repeated caps for one report ID are one report, not several.
    pub fn single_input_output_report_ids(
        &self,
        interface: &HidInterface,
        input: u16,
        output: u16,
    ) -> Option<(u8, u8)> {
        if self.caps.usage_page != interface.usage_page
            || self.caps.usage != interface.usage
            || input == 0
            || output == 0
            || self.caps.input_report_byte_length != input
            || self.caps.output_report_byte_length != output
        {
            return None;
        }
        Some((single_id(self.report_ids(0)?)?, single_id(self.report_ids(1)?)?))
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { (self.api.free_preparsed_data)(self.preparsed); CloseHandle(self.handle); }
    }
}

fn collect_ids(buttons: u16, values: u16,
    mut read: impl FnMut(bool, u16) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    if buttons > 512 || values > 512 || (buttons == 0 && values == 0) { return None; }
    let mut ids = Vec::new();
    for (is_button, count) in [(true, buttons), (false, values)] {
        if count == 0 { continue; }
        let entries = read(is_button, count)?;
        if entries.len() != usize::from(count) { return None; }
        ids.extend(entries);
    }
    Some(ids)
}

fn single_id(ids: Vec<u8>) -> Option<u8> {
    let first = *ids.first()?;
    ids.iter().all(|id| *id == first).then_some(first)
}

fn feature_byte_report_lengths(
    fields: &[HIDP_VALUE_CAPS],
    maximum: u16,
) -> Option<BTreeMap<u8, u16>> {
    if !(2..=4096).contains(&maximum) || fields.is_empty() || fields.len() > 32 {
        return None;
    }
    let mut sizes = BTreeMap::new();
    for field in fields {
        let length = field.ReportCount.checked_add(1)?;
        if field.IsAlias != 0
            || field.BitSize != 8
            || field.ReportCount == 0
            || length > maximum
            || sizes.insert(field.ReportID, length).is_some()
        {
            return None;
        }
    }
    (sizes.values().copied().max() == Some(maximum)).then_some(sizes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_fields_and_value_fields_are_both_required_and_zero_counts_are_not_queried() {
        let ids = collect_ids(2, 1, |buttons, count| {
            assert_eq!(count, if buttons { 2 } else { 1 });
            Some(if buttons { vec![4, 4] } else { vec![7] })
        }).unwrap();
        assert_eq!(ids, vec![4, 4, 7]);
        let ids = collect_ids(1, 0, |buttons, count| {
            assert!(buttons); assert_eq!(count, 1); Some(vec![4])
        }).unwrap();
        assert_eq!(ids, vec![4]);
        assert!(collect_ids(0, 0, |_, _| panic!("empty caps")).is_none());
        assert!(collect_ids(513, 0, |_, _| panic!("invalid caps")).is_none());
    }

    #[test]
    fn missing_or_partial_caps_do_not_prove_report_eligibility() {
        assert!(collect_ids(1, 1, |buttons, _| buttons.then_some(vec![4])).is_none());
        assert!(collect_ids(2, 0, |_, _| Some(vec![4])).is_none());
        assert!(collect_ids(0, 1, |_, _| None).is_none());
    }

    #[test]
    fn feature_report_proof_uses_each_report_id_and_requires_collection_maximum() {
        fn field(id: u8, bytes: u16) -> HIDP_VALUE_CAPS {
            let mut field: HIDP_VALUE_CAPS = unsafe { core::mem::zeroed() };
            field.ReportID = id;
            field.BitSize = 8;
            field.ReportCount = bytes - 1;
            field
        }
        let sizes = feature_byte_report_lengths(
            &[field(3, 64), field(9, 128)],
            128,
        )
        .unwrap();
        assert_eq!(sizes.get(&3), Some(&64));
        assert_eq!(sizes.get(&9), Some(&128));

        let mut alias = field(3, 64);
        alias.IsAlias = 1;
        let mut bits = field(3, 64);
        bits.BitSize = 16;
        assert!(feature_byte_report_lengths(&[alias], 64).is_none());
        assert!(feature_byte_report_lengths(&[bits], 64).is_none());
        assert!(feature_byte_report_lengths(&[field(3, 64), field(3, 64)], 64).is_none());
        assert!(feature_byte_report_lengths(&[field(3, 64)], 128).is_none());
        assert!(feature_byte_report_lengths(&[], 64).is_none());
    }

    #[test]
    fn report_id_uniqueness_collapses_repeated_caps_but_rejects_two_ids() {
        assert_eq!(single_id(vec![0, 0, 0]), Some(0));
        assert_eq!(single_id(vec![7, 7]), Some(7));
        assert_eq!(single_id(vec![]), None);
        assert_eq!(single_id(vec![0, 7]), None);
    }
}
