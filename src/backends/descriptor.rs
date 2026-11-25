//! Inspect report capabilities without opening an input or output stream.
use crate::{hid::HidInterface, win32::*};
use std::collections::BTreeMap;
use winapi::shared::hidpi::{HIDP_BUTTON_CAPS, HIDP_VALUE_CAPS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FeatureReportProof {
    Match(u8),
    Mismatch,
    Unknown,
}

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
        collect_ids(button_count, value_count, |buttons, count| self.report_cap_ids(kind, buttons, count))
    }

    fn report_cap_ids(&self, kind: u32, buttons: bool, count: u16) -> Option<Vec<u8>> {
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

    pub fn single_feature_report(&self, interface: &HidInterface, length: u16, allowed: &[u8])
        -> FeatureReportProof {
        feature_report_proof_with(&self.caps, interface, length, allowed,
            |buttons, count| self.report_cap_ids(2, buttons, count))
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

    /// Verify both directions of a vendor byte-array report. Other report IDs
    /// may share the collection, but cannot supply the required report's size.
    pub fn input_output_byte_reports(&self, interface: &HidInterface, id: u8,
        input: u16, output: u16) -> bool {
        if self.caps.usage_page != interface.usage_page || self.caps.usage != interface.usage
            || self.caps.input_report_byte_length != input
            || self.caps.output_report_byte_length != output { return false; }
        [(0, input, self.caps.number_input_button_caps, self.caps.number_input_value_caps),
         (1, output, self.caps.number_output_button_caps, self.caps.number_output_value_caps)]
            .into_iter().all(|(kind, length, buttons, count)| {
                if buttons != 0 || !(1..=32).contains(&count) { return false; }
                let mut values: Vec<HIDP_VALUE_CAPS> = (0..count)
                    .map(|_| unsafe { core::mem::zeroed() }).collect();
                let mut returned = count;
                if unsafe { (self.api.get_value_caps)(kind, values.as_mut_ptr(), &mut returned, self.preparsed) } < 0
                    || returned != count { return false; }
                feature_byte_report_lengths(&values, length)
                    .is_some_and(|sizes| sizes.get(&id) == Some(&length))
            })
    }

    /// Prove the sole input byte-array report without requiring an output side.
    pub fn single_input_byte_report_id(&self, interface: &HidInterface, length: u16) -> Option<u8> {
        if self.caps.usage_page != interface.usage_page || self.caps.usage != interface.usage
            || self.caps.input_report_byte_length != length
            || self.caps.number_input_button_caps != 0 { return None; }
        let count = self.caps.number_input_value_caps;
        if !(1..=32).contains(&count) { return None; }
        let mut values: Vec<HIDP_VALUE_CAPS> = (0..count)
            .map(|_| unsafe { core::mem::zeroed() }).collect();
        let mut returned = count;
        if unsafe { (self.api.get_value_caps)(0, values.as_mut_ptr(), &mut returned, self.preparsed) } < 0
            || returned != count { return None; }
        single_byte_report_id(&values, length)
    }

    /// Identify independently proven byte-array reports when the two directions
    /// use different IDs. Neither direction may contain a second report.
    pub fn single_input_output_byte_report_ids(&self, interface: &HidInterface,
        input: u16, output: u16) -> Option<(u8, u8)> {
        if self.caps.usage_page != interface.usage_page || self.caps.usage != interface.usage
            || self.caps.input_report_byte_length != input
            || self.caps.output_report_byte_length != output { return None; }
        let mut ids = [0; 2];
        for (kind, length, buttons, count) in [
            (0, input, self.caps.number_input_button_caps, self.caps.number_input_value_caps),
            (1, output, self.caps.number_output_button_caps, self.caps.number_output_value_caps),
        ] {
            if buttons != 0 || !(1..=32).contains(&count) { return None; }
            let mut values: Vec<HIDP_VALUE_CAPS> = (0..count)
                .map(|_| unsafe { core::mem::zeroed() }).collect();
            let mut returned = count;
            if unsafe { (self.api.get_value_caps)(kind, values.as_mut_ptr(), &mut returned, self.preparsed) } < 0
                || returned != count { return None; }
            ids[kind as usize] = single_byte_report_id(&values, length)?;
        }
        Some((ids[0], ids[1]))
    }

    /// For a caller's exact, independently documented Windows descriptor
    /// variant. Initializing a report only touches local memory, not the device.
    pub fn initialized_input_output_report(&self, interface: &HidInterface, id: u8, length: u16) -> bool {
        if self.caps.usage_page != interface.usage_page || self.caps.usage != interface.usage
            || !(2..=4096).contains(&length)
            || self.caps.input_report_byte_length != length
            || self.caps.output_report_byte_length != length
            || !(0..=1).all(|kind| self.report_ids(kind).is_some_and(|ids| ids.contains(&id))) {
            return false;
        }
        initialize_both_reports(id, length, |kind, buffer| unsafe {
            (self.api.initialize_report_for_id)(kind, id, self.preparsed, buffer.as_mut_ptr(), buffer.len() as u32)
        })
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

pub(super) fn feature_report_proof_with(caps: &HidpCaps, interface: &HidInterface,
    length: u16, allowed: &[u8], read: impl FnMut(bool, u16) -> Option<Vec<u8>>) -> FeatureReportProof {
    if length == 0 || caps.usage_page != interface.usage_page || caps.usage != interface.usage
        || caps.input_report_byte_length != interface.input_len
        || caps.output_report_byte_length != interface.output_len
        || caps.feature_report_byte_length != interface.feature_len {
        return FeatureReportProof::Unknown;
    }
    let Some(ids) = collect_ids(caps.number_feature_button_caps, caps.number_feature_value_caps, read)
        else { return FeatureReportProof::Unknown; };
    match single_id(ids) {
        Some(report_id) if caps.feature_report_byte_length == length && allowed.contains(&report_id)
            => FeatureReportProof::Match(report_id),
        _ => FeatureReportProof::Mismatch,
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

fn initialize_both_reports(id: u8, length: u16, mut initialize: impl FnMut(u32, &mut [u8]) -> i32) -> bool {
    (0..=1).all(|kind| {
        let mut buffer = vec![0; usize::from(length)];
        initialize(kind, &mut buffer) >= 0 && buffer.first() == Some(&id)
    })
}

fn single_byte_report_id(fields: &[HIDP_VALUE_CAPS], maximum: u16) -> Option<u8> {
    let reports = feature_byte_report_lengths(fields, maximum)?;
    (reports.len() == 1).then(|| *reports.keys().next().unwrap())
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

    fn feature_fixture(buttons: u16, values: u16) -> (HidInterface, HidpCaps) {
        let interface = HidInterface {
            path: "feature-proof-fixture".into(), serial: String::new(), product: String::new(),
            vendor_id: 0x3151, product_id: 0x5026, usage_page: 0xffff, usage: 2,
            input_len: 0, output_len: 0, feature_len: 65,
        };
        let caps = HidpCaps {
            usage: interface.usage, usage_page: interface.usage_page,
            feature_report_byte_length: interface.feature_len,
            number_feature_button_caps: buttons, number_feature_value_caps: values,
            ..Default::default()
        };
        (interface, caps)
    }

    #[test]
    fn feature_proof_reads_all_caps_and_collapses_repeated_supported_ids() {
        for report_id in [0, 0x83] {
            for (buttons, values) in [(2, 1), (2, 0), (0, 1)] {
                let (interface, caps) = feature_fixture(buttons, values);
                let mut calls = Vec::new();
                let proof = feature_report_proof_with(&caps, &interface, 65, &[0x83, 0],
                    |is_button, count| {
                        calls.push((is_button, count));
                        Some(vec![report_id; usize::from(count)])
                    });
                assert_eq!(proof, FeatureReportProof::Match(report_id));
                assert_eq!(calls, [(true, buttons), (false, values)].into_iter()
                    .filter(|(_, count)| *count != 0).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn complete_unsupported_or_multiple_feature_ids_are_mismatches() {
        let (interface, caps) = feature_fixture(1, 1);
        for (button_id, value_id) in [(0x84, 0x84), (0, 0x83), (0x83, 0x84)] {
            let mut calls = 0;
            assert_eq!(feature_report_proof_with(&caps, &interface, 65, &[0x83, 0],
                |is_button, _| {
                    calls += 1;
                    Some(vec![if is_button { button_id } else { value_id }])
                }), FeatureReportProof::Mismatch);
            assert_eq!(calls, 2);
        }
    }

    #[test]
    fn failed_or_partial_feature_caps_are_unknown_even_after_an_unsupported_id() {
        let (interface, caps) = feature_fixture(1, 1);
        for failed_buttons in [false, true] {
            for incomplete in [None, Some(vec![]), Some(vec![0x84, 0x84])] {
                assert_eq!(feature_report_proof_with(&caps, &interface, 65, &[0x83, 0],
                    |is_button, _| if is_button == failed_buttons { incomplete.clone() }
                        else { Some(vec![0x84]) }), FeatureReportProof::Unknown);
            }
        }
    }

    #[test]
    fn inconsistent_feature_metadata_never_proves_a_mismatch() {
        for mutate in [
            |caps: &mut HidpCaps| caps.usage ^= 1,
            |caps: &mut HidpCaps| caps.usage_page ^= 1,
            |caps: &mut HidpCaps| caps.input_report_byte_length = 65,
            |caps: &mut HidpCaps| caps.output_report_byte_length = 65,
            |caps: &mut HidpCaps| caps.feature_report_byte_length = 64,
        ] {
            let (interface, mut caps) = feature_fixture(1, 1);
            mutate(&mut caps);
            assert_eq!(feature_report_proof_with(&caps, &interface, 65, &[0x83, 0],
                |_, _| panic!("inconsistent metadata")), FeatureReportProof::Unknown);
        }
    }

    #[test]
    fn empty_or_excessive_feature_capability_counts_remain_unknown() {
        for (buttons, values) in [(0, 0), (513, 0), (0, 513), (1, 513)] {
            let (interface, caps) = feature_fixture(buttons, values);
            assert_eq!(feature_report_proof_with(&caps, &interface, 65, &[0x83, 0],
                |_, _| panic!("unsupported capability counts")), FeatureReportProof::Unknown);
        }
    }

    #[test]
    fn windows_report_initialization_must_confirm_both_directions_and_the_requested_id() {
        let mut calls = Vec::new();
        assert!(initialize_both_reports(0x4d, 64, |kind, r| {
            calls.push(kind); assert_eq!(r.len(), 64); r[0] = 0x4d; 0x00110000
        }));
        assert_eq!(calls, vec![0, 1]);
        for denied in [0, 1] {
            assert!(!initialize_both_reports(0x4d, 64, |kind, r| {
                r[0] = 0x4d;
                if kind == denied { 0xc0110010u32 as i32 } else { 0x00110000 }
            }));
        }
        assert!(!initialize_both_reports(0x4d, 64, |_, _| 0x00110000));
    }

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

    #[test]
    fn byte_report_identity_requires_a_complete_array_and_rejects_other_ids() {
        let mut input: HIDP_VALUE_CAPS = unsafe { core::mem::zeroed() };
        input.ReportID = 0;
        input.BitSize = 8;
        input.ReportCount = 15;
        let mut output = input;
        output.ReportID = 8;
        output.ReportCount = 63;
        assert_eq!(single_byte_report_id(&[input], 16), Some(0));
        assert_eq!(single_byte_report_id(&[output], 64), Some(8));
        assert_eq!(single_byte_report_id(&[input, output], 64), None);
        assert_eq!(single_byte_report_id(&[input], 64), None);
        output.BitSize = 1;
        assert_eq!(single_byte_report_id(&[output], 64), None);
    }
}
