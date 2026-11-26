use super::feedback::text;
use super::gfx::{Align, Painter, R};
use super::pages::button;
use super::widget::{Act, Ui};
use crate::store::{Failure, Phase, View};

fn title(phase: Phase) -> &'static str {
    match phase {
        Phase::Unsupported => text("微软商店更新", "Microsoft Store updates"),
        Phase::Idle => text("更新状态尚未确认", "Update status not yet confirmed"),
        Phase::Checking => text("正在检查微软商店更新…", "Checking Microsoft Store…"),
        Phase::Available => text(
            "微软商店有可用更新",
            "An update is available in Microsoft Store",
        ),
        Phase::Current => text(
            "微软商店暂无可用更新",
            "No updates available in Microsoft Store",
        ),
        Phase::CheckFailed => text("暂时无法检查更新", "Unable to check for updates"),
        Phase::AwaitingPreparation | Phase::Preparing => text("正在准备更新…", "Preparing update…"),
        Phase::Installing => text(
            "正在下载并安装更新…",
            "Downloading and installing update…",
        ),
        Phase::Installed | Phase::Restarting => text("正在重启…", "Restarting…"),
        Phase::RestartFailed => text("更新已安装，重启失败", "Update installed; restart failed"),
        Phase::Canceled => text("本次更新操作已取消", "Update operation canceled"),
        Phase::UpdateFailed => text("本次更新未完成", "Update did not complete"),
    }
}

fn description(view: &View) -> String {
    let message = match view.phase {
        Phase::Unsupported => "",
        Phase::Idle => text("点击“检查更新”获取更新状态。", "Select Check for updates to get the update status."),
        Phase::Checking | Phase::Current => "",
        Phase::Available => text("更新完成后自动重启。", "Restarts automatically after updating."),
        Phase::CheckFailed | Phase::UpdateFailed | Phase::RestartFailed => text("请稍后重试。", "Try again later."),
        Phase::AwaitingPreparation | Phase::Preparing | Phase::Installing | Phase::Installed | Phase::Restarting | Phase::Canceled => "",
    };
    let detail = match view.failure {
        Some(Failure::Windows(code)) => format!(" (0x{:08X})", code as u32),
        Some(Failure::TimedOut) => {
            format!(" {}", text("商店查询超时。", "The Store query timed out."))
        }
        Some(Failure::LowBattery) => format!(
            " {}",
            text("请先为电脑充电。", "Charge your computer first.")
        ),
        Some(Failure::WifiRecommended) => format!(
            " {}",
            text("商店建议使用 Wi-Fi。", "The Store recommends Wi-Fi.")
        ),
        Some(Failure::WifiRequired) => format!(
            " {}",
            text("商店要求使用 Wi-Fi。", "The Store requires Wi-Fi.")
        ),
        _ => String::new(),
    };
    format!("{message}{detail}")
}

fn actions(view: &View, home: bool) -> Vec<(&'static str, Act, bool)> {
    let mut actions = Vec::new();
    if view.phase == Phase::Unsupported {
        return actions;
    }
    match view.phase {
        Phase::RestartFailed => actions.push((text("重启软件", "Restart app"), Act::RestartUpdatedApp, true)),
        Phase::Available | Phase::Canceled | Phase::CheckFailed | Phase::UpdateFailed
            if view.available =>
        {
            actions.push((
                text("立即更新", "Update now"),
                Act::UpdateNow,
                true,
            ));
        }
        Phase::Checking | Phase::AwaitingPreparation | Phase::Preparing => {
            actions.push((text("取消", "Cancel"), Act::CancelUpdate, false));
        }
        _ => {}
    }
    if !matches!(
        view.phase,
        Phase::Preparing | Phase::Installing | Phase::Installed | Phase::Restarting
    ) {
        actions.push((
            text("商店更新说明", "Store release information"),
            Act::OpenStore,
            false,
        ));
    }
    if !home
        && !view.phase.busy()
        && matches!(
            view.phase,
            Phase::Available
                | Phase::CheckFailed
                | Phase::UpdateFailed
                | Phase::Canceled
        )
    {
        actions.push((
            text("退出并在商店更新", "Exit and update in Store"),
            Act::ExitToStoreUpdate,
            false,
        ));
    }
    if home && view.home_visible && !view.phase.busy() && view.phase != Phase::Installed {
        actions.push((text("稍后", "Later"), Act::DismissUpdate, false));
    }
    actions
}

pub fn dialog(painter: &mut Painter, ui: &mut Ui, width: f32, height: f32) {
    let theme = painter.t;
    painter.rect(R::new(0.0, 0.0, width, height), 0x7500_0000);
    let dialog_width = if crate::i18n::english() { 480.0 } else { 400.0 };
    let area = R::new((width - dialog_width) / 2.0, (height - 190.0) / 2.0, dialog_width, 190.0);
    painter.card(area, 12.0, theme.card, theme.border);
    painter.text(text("发现新版本", "Update available"),
        R::new(area.x + 24.0, area.y + 22.0, area.w - 48.0, 26.0),
        16.0, true, theme.text, Align::Left);
    painter.text_wrap(text("立即更新 X-Battery，完成后自动重启。", "Update X-Battery and restart automatically."),
        R::new(area.x + 24.0, area.y + 60.0, area.w - 48.0, 48.0),
        13.0, theme.text2);
    let update = R::new(area.right() - 140.0, area.bottom() - 58.0, 116.0, 34.0);
    let later = R::new(update.x - 100.0, update.y, 88.0, update.h);
    for (rect, label, action, primary) in [
        (later, text("稍后", "Later"), Act::DismissUpdate, false),
        (update, text("立即更新", "Update now"), Act::UpdateNow, true),
    ] {
        painter.round(rect, 6.0, if primary { theme.accent } else { theme.card });
        if !primary { painter.round_stroke(rect, 6.0, theme.border, 1.0); }
        painter.text(label, rect, 13.0, primary,
            if primary { 0xFFFF_FFFF } else { theme.text }, Align::Center);
        ui.dialog_hot(rect, action);
    }
}

pub fn card(painter: &mut Painter, ui: &mut Ui, view: &View, area: R, home: bool) -> f32 {
    if view.phase == Phase::Unsupported {
        return 0.0;
    }
    let theme = painter.t;
    let controls = actions(view, home);
    let description = description(view);
    let text_width = (area.w - 36.0).max(100.0);
    let mut control_rects = Vec::new();
    let mut control_x = area.x + 18.0;
    let mut control_y = area.y + if description.is_empty() { 55.0 } else { 111.0 };
    for (label, action, primary) in controls {
        let width = (painter.measure(label, 12.5, primary).0 + 28.0)
            .max(72.0)
            .min(text_width);
        if control_x > area.x + 18.0 && control_x + width > area.right() - 18.0 {
            control_x = area.x + 18.0;
            control_y += 44.0;
        }
        let rect = R::new(control_x, control_y, width, 34.0);
        control_rects.push((rect, label, action, primary));
        control_x += width + 10.0;
    }
    let height = if control_rects.is_empty() {
        if description.is_empty() { 56.0 } else { 112.0 }
    } else {
        control_y - area.y + 52.0
    };
    let card = R::new(area.x, area.y, area.w, height);
    painter.card(
        card,
        12.0,
        theme.card,
        if home { theme.accent } else { theme.border },
    );
    painter.text(
        title(view.phase),
        R::new(card.x + 18.0, card.y + 14.0, text_width, 25.0),
        14.0,
        true,
        if view.failure.is_some() {
            theme.warn
        } else {
            theme.text
        },
        Align::Left,
    );
    painter.text_wrap(
        &description,
        R::new(card.x + 18.0, card.y + 43.0, text_width, 56.0),
        12.0,
        theme.text2,
    );
    for (rect, label, action, primary) in control_rects {
        button(painter, ui, rect, label, primary, action);
    }
    if !home {
        if let Some(day) = view.last_checked_day {
            let checked = format!(
                "{} {:04}-{:02}-{:02}",
                text("最近成功检查：", "Last successful check:"),
                day / 10000,
                day / 100 % 100,
                day % 100
            );
            painter.text(
                &checked,
                R::new(area.x + 2.0, area.y + height + 10.0, area.w - 4.0, 18.0),
                11.5,
                false,
                theme.text3,
                Align::Left,
            );
            return height + 28.0;
        }
    }
    height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(phase: Phase, available: bool) -> View {
        View {
            phase,
            available,
            home_visible: available,
            failure: None,
            last_checked_day: None,
        }
    }

    #[test]
    fn available_update_has_one_action_and_no_second_install_step() {
        let offered = actions(&view(Phase::Available, true), true);
        assert!(offered
            .iter()
            .any(|(_, action, _)| *action == Act::UpdateNow));
        for phase in [Phase::AwaitingPreparation, Phase::Preparing, Phase::Installing, Phase::Installed] {
            assert!(!actions(&view(phase, true), true).iter()
                .any(|(_, action, _)| matches!(action, Act::UpdateNow | Act::DismissUpdate)));
        }
    }

    #[test]
    fn unsupported_and_failed_checks_never_offer_unconfirmed_downloads() {
        for phase in [
            Phase::Unsupported,
            Phase::Idle,
            Phase::Current,
            Phase::CheckFailed,
        ] {
            assert!(!actions(&view(phase, false), false)
                .iter()
                .any(|(_, action, _)| *action == Act::UpdateNow));
        }
    }

    #[test]
    fn unsupported_builds_have_no_store_update_controls() {
        for home in [false, true] {
            assert!(actions(&view(Phase::Unsupported, true), home).is_empty());
        }
    }

    #[test]
    fn official_install_cannot_be_interrupted_by_an_in_app_cancel() {
        assert!(actions(&view(Phase::Installing, true), false).is_empty());
        assert!(actions(&view(Phase::Preparing, true), false)
            .iter()
            .any(|(_, action, _)| *action == Act::CancelUpdate));
    }

    #[test]
    fn postponing_is_only_offered_on_the_home_card() {
        assert!(actions(&view(Phase::Available, true), true)
            .iter()
            .any(|(_, action, _)| *action == Act::DismissUpdate));
        assert!(!actions(&view(Phase::Available, true), false)
            .iter()
            .any(|(_, action, _)| *action == Act::DismissUpdate));
    }
}
