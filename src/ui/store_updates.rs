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
        Phase::WaitingToDownload => text(
            "更新已确认，等待开始下载",
            "Update confirmed; waiting to download",
        ),
        Phase::Downloading => text(
            "正在通过微软商店下载…",
            "Downloading through Microsoft Store…",
        ),
        Phase::ReadyToInstall => text(
            "更新已下载，可以安装",
            "Update downloaded and ready to install",
        ),
        Phase::Preparing => text(
            "正在等待设备操作完成并保存数据…",
            "Waiting for device operations and saving data…",
        ),
        Phase::Installing => text(
            "正在通过微软商店安装…",
            "Installing through Microsoft Store…",
        ),
        Phase::Installed => text("更新已安装", "Update installed"),
        Phase::Canceled => text("本次更新操作已取消", "Update operation canceled"),
        Phase::UpdateFailed => text("本次更新未完成", "Update did not complete"),
    }
}

fn description(view: &View) -> String {
    let message = match view.phase {
        Phase::Unsupported => text(
            "应用内更新仅适用于微软商店安装版；绿色版和模拟设备模式不会自动检查。",
            "In-app updates require a Microsoft Store installation. Portable and simulated-device builds do not check automatically.",
        ),
        Phase::Idle => text("点击“检查更新”获取更新状态。", "Select Check for updates to get the update status."),
        Phase::Checking | Phase::Current => "",
        Phase::Available => text("更新说明可在微软商店查看。", "View release information in Microsoft Store."),
        Phase::CheckFailed | Phase::UpdateFailed => text("请检查网络或稍后重试，也可以退出软件后在微软商店更新。", "Check your connection or try later. You can also exit the app and update in Microsoft Store."),
        Phase::WaitingToDownload => text("回到主窗口后才会调用商店下载；此时仍可取消。", "Store download starts only while the main window is active. You can still cancel."),
        Phase::Downloading => text("下载完成后，可选择安装更新。", "You can install the update once the download finishes."),
        Phase::ReadyToInstall => text("点击“安装更新”继续，软件可能关闭或重启。", "Select Install update to continue. The app may close or restart."),
        Phase::Preparing => text("正在准备安装，请稍候。", "Preparing to install. Please wait."),
        Phase::Installing => text("请在微软商店的官方提示中确认或取消。Windows 可能关闭并重启软件。", "Confirm or cancel in the official Microsoft Store prompt. Windows may close and restart the app."),
        Phase::Installed => text("软件即将退出；如 Windows 未自动重启软件，请更新后重新打开。", "The app will exit. If Windows does not restart it, open it again after the update."),
        Phase::Canceled => text("已取消更新，可稍后重试。", "Update canceled. You can try again later."),
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
        Phase::Available | Phase::Canceled | Phase::CheckFailed | Phase::UpdateFailed
            if view.available =>
        {
            actions.push((
                text("下载更新", "Download update"),
                Act::DownloadUpdate,
                true,
            ));
        }
        Phase::ReadyToInstall => {
            actions.push((text("安装更新", "Install update"), Act::InstallUpdate, true))
        }
        Phase::Checking | Phase::WaitingToDownload | Phase::Downloading | Phase::Preparing => {
            actions.push((text("取消", "Cancel"), Act::CancelUpdate, false));
        }
        _ => {}
    }
    if !matches!(
        view.phase,
        Phase::Preparing | Phase::Installing | Phase::Installed
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
                | Phase::ReadyToInstall
                | Phase::Canceled
        )
    {
        actions.push((
            text("退出并在商店更新", "Exit and update in Store"),
            Act::ExitToStoreUpdate,
            false,
        ));
    }
    if home && view.home_visible {
        actions.push((text("稍后提醒", "Later today"), Act::DismissUpdate, false));
    }
    actions
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
    fn download_and_install_are_separate_explicit_actions() {
        let offered = actions(&view(Phase::Available, true), true);
        assert!(offered
            .iter()
            .any(|(_, action, _)| *action == Act::DownloadUpdate));
        assert!(!offered
            .iter()
            .any(|(_, action, _)| *action == Act::InstallUpdate));
        let downloaded = actions(&view(Phase::ReadyToInstall, true), true);
        assert!(downloaded
            .iter()
            .any(|(_, action, _)| *action == Act::InstallUpdate));
        assert!(!downloaded
            .iter()
            .any(|(_, action, _)| *action == Act::DownloadUpdate));
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
                .any(|(_, action, _)| *action == Act::DownloadUpdate));
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
