use super::R;

pub(super) type PixelRect = (i32, i32, i32, i32);

pub(super) struct NativeState {
    rect: Option<PixelRect>,
    text: String,
    visible: bool,
    enabled: bool,
}

impl Default for NativeState {
    fn default() -> Self {
        Self { rect: None, text: String::new(), visible: false, enabled: true }
    }
}

#[derive(Default, Debug, PartialEq)]
pub(super) struct NativeChanges {
    pub(super) rect: Option<PixelRect>,
    pub(super) text: Option<String>,
    pub(super) visible: Option<bool>,
    pub(super) enabled: Option<bool>,
}

impl NativeState {
    pub(super) fn update(&mut self, rect: Option<PixelRect>, text: Option<&str>, visible: bool,
        enabled: bool) -> NativeChanges {
        let mut changes = NativeChanges::default();
        if rect.is_some() && self.rect != rect {
            self.rect = rect;
            changes.rect = rect;
        }
        if let Some(text) = text {
            if self.text != text {
                self.text = text.into();
                changes.text = Some(self.text.clone());
            }
        }
        if self.visible != visible {
            self.visible = visible;
            changes.visible = Some(visible);
        }
        if self.enabled != enabled {
            self.enabled = enabled;
            changes.enabled = Some(enabled);
        }
        changes
    }
}

pub(super) fn pixels(rect: R, scale: f32) -> PixelRect {
    ((rect.x * scale).round() as i32, (rect.y * scale).round() as i32,
        (rect.w * scale).round() as i32, (rect.h * scale).round() as i32)
}

pub(super) fn intersects(rect: R, damage: R) -> bool {
    rect.x < damage.right() && rect.right() > damage.x
        && rect.y < damage.bottom() && rect.bottom() > damage.y
}

pub(super) fn buffer_size(previous: (i32, i32), requested: (i32, i32)) -> (i32, i32) {
    (previous.0.max(requested.0).max(1), previous.1.max(requested.1).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_controls_do_not_move_hide_relabel_or_disable_again() {
        let mut state = NativeState::default();
        let rect = Some((25, 91, 610, 103));
        state.update(rect, Some("Feedback"), true, true);
        for _iteration in 0..128 {
            assert_eq!(state.update(rect, Some("Feedback"), true, true), NativeChanges::default());
        }
    }

    #[test]
    fn scrolling_moves_a_control_without_changing_text_focus_or_visibility() {
        let mut state = NativeState::default();
        state.update(Some((25, 91, 610, 103)), Some("Feedback"), true, true);
        assert_eq!(state.update(Some((25, 25, 610, 103)), Some("Feedback"), true, true),
            NativeChanges { rect: Some((25, 25, 610, 103)), ..Default::default() });
    }

    #[test]
    fn idle_hidden_slots_do_not_issue_repeated_hide_calls() {
        let mut state = NativeState::default();
        assert_eq!(state.update(None, None, false, true), NativeChanges::default());
        state.update(Some((25, 91, 610, 103)), None, true, true);
        assert_eq!(state.update(None, None, false, true),
            NativeChanges { visible: Some(false), ..Default::default() });
        assert_eq!(state.update(None, None, false, true), NativeChanges::default());
    }

    #[test]
    fn sending_only_changes_the_editor_enabled_state() {
        let mut state = NativeState::default();
        state.update(Some((37, 102, 586, 85)), None, true, true);
        assert_eq!(state.update(Some((37, 102, 586, 85)), None, true, false),
            NativeChanges { enabled: Some(false), ..Default::default() });
    }

    #[test]
    fn button_buffers_do_not_shrink_between_parent_paints_or_small_controls() {
        let mut size = buffer_size((0, 0), (610, 103));
        for _iteration in 0..128 {
            size = buffer_size(size, (90, 32));
            assert_eq!(size, (610, 103));
        }
        assert_eq!(buffer_size(size, (1220, 206)), (1220, 206));
    }

    #[test]
    fn count_repaints_do_not_include_unrelated_labels_or_offscreen_replies() {
        let count = R::new(540.0, 350.0, 83.0, 19.0);
        assert!(intersects(count, count));
        assert!(!intersects(R::new(25.0, 25.0, 400.0, 19.0), count));
        assert!(!intersects(R::new(25.0, 4000.0, 610.0, 200.0), R::new(0.0, 0.0, 660.0, 500.0)));
    }

    #[test]
    fn pixel_geometry_accounts_for_scaling_and_negative_scroll_positions() {
        assert_eq!(pixels(R::new(25.0, -11.0, 610.0, 103.0), 1.5), (38, -17, 915, 155));
    }
}
