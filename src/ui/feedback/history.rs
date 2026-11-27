use super::{client, text, wu, Action, Controls, View};
use winapi::shared::windef::{HMENU, HWND, POINT};

struct Menu(HMENU);

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe { wu::DestroyMenu(self.0); }
    }
}

pub(super) fn actions(entry: &client::Entry) -> Vec<(&'static str, Action)> {
    let mut actions = vec![(text("导出文件", "Export files"), Action::Export(Some(entry.request_id.clone())))];
    if entry.status == client::Status::Pending {
        actions.push((text("重新发送", "Retry"), Action::RetryEntry(entry.request_id.clone())));
    }
    actions.push((text("删除", "Delete"), Action::DeleteLocal(entry.request_id.clone())));
    actions
}

impl Controls {
    pub(super) fn history_context_target(&self, source: HWND, mut point: POINT, keyboard: bool)
        -> Option<(client::Entry, POINT)> {
        let model = self.state.borrow();
        if model.view != View::History || model.busy() { return None; }
        let body = self.body.handle.hwnd()?;
        let scale = self.scale.get();
        let mut local = point;
        if !keyboard {
            let mut viewport = unsafe { std::mem::zeroed() };
            unsafe {
                if wu::ScreenToClient(body, &mut local) == 0 || wu::GetClientRect(body, &mut viewport) == 0 {
                    return None;
                }
            }
            if local.x < 0 || local.y < 0 || local.x >= viewport.right || local.y >= viewport.bottom {
                return None;
            }
        }
        let items = self.items.borrow();
        for (slot, item) in items.iter().filter(|item| !item.root).enumerate() {
            let Action::Entry(index) = item.action else { continue; };
            let top = item.rect.y - self.scroll.get();
            let bottom = top + item.rect.h;
            if bottom <= 0.0 || top >= self.body_height() { continue; }
            let selected = if keyboard {
                self.buttons.get(slot)?.handle.hwnd() == Some(source)
            } else {
                let mut displayed = item.rect;
                displayed.y = top;
                let (left, top, width, height) = super::layout::pixels(displayed, scale);
                local.x >= left && local.x < left + width && local.y >= top && local.y < top + height
            };
            if !selected { continue; }
            if keyboard {
                point = POINT {
                    x: ((item.rect.x + 16.0) * scale).round() as i32,
                    y: ((top.max(0.0) + bottom.min(self.body_height())) * 0.5 * scale).round() as i32,
                };
                if unsafe { wu::ClientToScreen(body, &mut point) } == 0 { return None; }
            }
            return Some((model.entries.get(index)?.clone(), point));
        }
        None
    }

    pub(super) fn history_context_menu(&self, source: HWND, position: isize) -> bool {
        let keyboard = position == -1;
        let point = POINT { x: position as i16 as i32, y: (position >> 16) as i16 as i32 };
        let Some((entry, point)) = self.history_context_target(source, point, keyboard) else { return false; };
        let actions = actions(&entry);
        let handle = unsafe { wu::CreatePopupMenu() };
        if handle.is_null() { return true; }
        let menu = Menu(handle);
        for (index, (label, _)) in actions.iter().enumerate() {
            let label = crate::win32::to_wide(label);
            if unsafe { wu::AppendMenuW(menu.0, wu::MF_STRING, index + 1, label.as_ptr()) } == 0 {
                return true;
            }
        }
        let command = unsafe {
            wu::TrackPopupMenu(menu.0, wu::TPM_RETURNCMD | wu::TPM_NONOTIFY | wu::TPM_RIGHTBUTTON,
                point.x, point.y, 0, self.window.handle.hwnd().unwrap(), std::ptr::null())
        };
        drop(menu);
        if let Some((_, action)) = command.checked_sub(1).and_then(|index| actions.get(index as usize)) {
            self.action(action.clone());
        }
        true
    }
}
