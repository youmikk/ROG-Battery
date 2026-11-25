use std::fmt::Write;

use crate::snapshot::FetchOutcome;

pub(super) fn render(outcome: &FetchOutcome, cache_age_sec: u64) -> String {
    let snapshot = &outcome.snapshot;
    let state = &snapshot.state;
    let energy = snapshot.parsed_frame("12:07:00");
    let performance = snapshot.parsed_frame("12:04:00");
    let profile = snapshot.parsed_frame("12:00:00");
    let percent = snapshot.battery().and_then(|battery| battery.percent);
    let complete = energy && performance && profile && percent.is_some();
    let mut output = String::with_capacity(384);
    let _ = writeln!(output, "state={}", if complete { "ok" } else { "partial" });
    if let Some(percent) = percent {
        let _ = writeln!(output, "level={percent}");
    }
    if energy {
        let _ = writeln!(output, "voltage_mv={}", state.voltage_mv);
        let _ = writeln!(output, "charging={}", state.charging);
        let _ = writeln!(output, "sleep_mode={}", state.sleep_mode);
        let _ = writeln!(output, "low_warn={}", state.low_warn);
        let _ = writeln!(output, "full_effect={}", state.full_effect);
    }
    if performance {
        let _ = writeln!(output, "poll={}", crate::proto::poll_name(state.poll_code));
        let _ = writeln!(output, "poll_code={}", state.poll_code);
        let _ = writeln!(output, "debounce_code={}", state.debounce_code);
        let _ = writeln!(output, "angle_snap={}", state.angle_snap);
        let _ = writeln!(output, "angle_tuning={}", state.angle_tuning);
    }
    if profile {
        let _ = writeln!(output, "active_stage={}", state.active_stage);
        let _ = writeln!(output, "profile={}", state.profile);
    }
    let _ = writeln!(output, "frames={}", outcome.frames_read);
    if outcome.elapsed.is_zero() {
        let _ = writeln!(output, "cached=yes");
        let _ = writeln!(output, "cache_age_sec={cache_age_sec}");
    } else {
        let _ = writeln!(output, "cached=no");
        let _ = writeln!(output, "elapsed_ms={}", outcome.elapsed.as_millis());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn outcome_with(mut reply: impl FnMut(u8, Vec<u8>) -> Option<Vec<u8>>) -> FetchOutcome {
        let mut outcome =
            crate::snapshot::fetch_light_with(3, "Wireless24G", |rid, cmd, sub, tail| {
                let mut frame = vec![0; 64];
                frame[..3].copy_from_slice(&[rid, cmd, sub]);
                frame[3..5].copy_from_slice(tail);
                match sub {
                    0x07 => {
                        frame[6] = 2;
                        frame[7] = 20;
                        frame[8..10].copy_from_slice(&3900u16.to_le_bytes());
                        frame[10] = 2;
                        frame[11] = 1;
                        frame[12] = 1;
                    }
                    0x04 => {
                        frame[13] = 3;
                        frame[15] = 2;
                        frame[17] = 1;
                        frame[19..21].copy_from_slice(&(-5i16).to_le_bytes());
                    }
                    0x00 => {
                        frame[11] = 2;
                        frame[12] = 4;
                    }
                    _ => panic!("unexpected quick frame {sub}"),
                }
                reply(sub, frame)
            });
        outcome.elapsed = Duration::from_millis(17);
        outcome
    }

    fn fields(output: &str) -> std::collections::BTreeMap<&str, &str> {
        output
            .lines()
            .map(|line| line.split_once('=').unwrap())
            .collect()
    }

    #[test]
    fn complete_zero_battery_keeps_existing_keys_values_and_order() {
        let outcome = outcome_with(|_, frame| Some(frame));
        assert_eq!(
            render(&outcome, 99),
            concat!(
                "state=ok\nlevel=0\nvoltage_mv=3900\ncharging=2\n",
                "sleep_mode=2\nlow_warn=20\nfull_effect=1\n",
                "poll=1000Hz\npoll_code=3\ndebounce_code=2\nangle_snap=1\nangle_tuning=-5\n",
                "active_stage=4\nprofile=2\nframes=3\ncached=no\nelapsed_ms=17\n",
            )
        );
    }

    #[test]
    fn short_frames_never_render_default_fields() {
        for (sub, key, omitted) in [
            (
                0x07,
                "12:07:00",
                &[
                    "level",
                    "voltage_mv",
                    "charging",
                    "sleep_mode",
                    "low_warn",
                    "full_effect",
                ][..],
            ),
            (
                0x04,
                "12:04:00",
                &[
                    "poll",
                    "poll_code",
                    "debounce_code",
                    "angle_snap",
                    "angle_tuning",
                ][..],
            ),
            (0x00, "12:00:00", &["active_stage", "profile"][..]),
        ] {
            let outcome = outcome_with(|requested, mut frame| {
                if requested == sub {
                    frame.truncate(3);
                }
                Some(frame)
            });
            assert!(outcome.snapshot.failed.contains_key(key));
            let output = render(&outcome, 0);
            let fields = fields(&output);
            assert_eq!(fields["state"], "partial");
            assert_eq!(fields["frames"], "3");
            for field in omitted {
                assert!(!fields.contains_key(field), "{key}: {field}");
            }
            if sub != 0x07 {
                assert_eq!(fields["level"], "0");
            }
            if sub != 0x04 {
                assert_eq!(fields["poll"], "1000Hz");
            }
            if sub != 0x00 {
                assert_eq!(fields["active_stage"], "4");
            }
        }
    }

    #[test]
    fn missing_performance_or_profile_preserves_valid_fields() {
        for (sub, absent, retained, value) in [
            (0x04, "poll_code", "profile", "2"),
            (0x00, "profile", "poll_code", "3"),
        ] {
            let outcome = outcome_with(|requested, frame| (requested != sub).then_some(frame));
            let output = render(&outcome, 0);
            let fields = fields(&output);
            assert_eq!(fields["state"], "partial");
            assert_eq!(fields["level"], "0");
            assert_eq!(fields["frames"], "2");
            assert_eq!(fields[retained], value);
            assert!(!fields.contains_key(absent));
        }
    }

    #[test]
    fn rejected_energy_preserves_performance_and_profile() {
        let outcome = outcome_with(|sub, frame| {
            Some(if sub == 0x07 {
                vec![3, 0xff, 0xaa]
            } else {
                frame
            })
        });
        let output = render(&outcome, 0);
        let fields = fields(&output);
        assert_eq!(fields["state"], "partial");
        assert_eq!(fields["frames"], "2");
        assert_eq!(fields["poll"], "1000Hz");
        assert_eq!(fields["profile"], "2");
        assert!(!fields.contains_key("level"));
        assert!(!fields.contains_key("voltage_mv"));
    }

    #[test]
    fn invalid_battery_percent_is_omitted_without_losing_parsed_energy_fields() {
        for percent in [101, 255] {
            let outcome = outcome_with(|sub, mut frame| {
                if sub == 0x07 {
                    frame[5] = percent;
                }
                Some(frame)
            });
            let output = render(&outcome, 0);
            let fields = fields(&output);
            assert_eq!(fields["state"], "partial");
            assert!(!fields.contains_key("level"));
            assert_eq!(fields["voltage_mv"], "3900");
            assert_eq!(fields["sleep_mode"], "2");
            assert_eq!(fields["poll_code"], "3");
        }
    }

    #[test]
    fn cached_output_keeps_explicit_age_and_frame_statistics() {
        let mut outcome = outcome_with(|_, frame| Some(frame));
        outcome.elapsed = Duration::ZERO;
        outcome.frames_read = 19;
        let output = render(&outcome, 123);
        let fields = fields(&output);
        assert_eq!(fields["state"], "ok");
        assert_eq!(fields["cached"], "yes");
        assert_eq!(fields["cache_age_sec"], "123");
        assert_eq!(fields["frames"], "19");
        assert!(!fields.contains_key("elapsed_ms"));
        assert_eq!(output, render(&outcome, 123));
    }

    #[test]
    fn cached_invalid_energy_remains_partial() {
        let mut outcome = outcome_with(|sub, mut frame| {
            if sub == 0x07 {
                frame.truncate(3);
            }
            Some(frame)
        });
        outcome.elapsed = Duration::ZERO;
        let output = render(&outcome, 7);
        let fields = fields(&output);
        assert_eq!(fields["state"], "partial");
        assert_eq!(fields["cached"], "yes");
        assert_eq!(fields["cache_age_sec"], "7");
        assert!(!fields.contains_key("level"));
        assert_eq!(fields["active_stage"], "4");
    }

    #[test]
    fn empty_snapshot_never_reports_ok_or_fabricated_values() {
        let outcome = outcome_with(|_, _| None);
        assert_eq!(
            render(&outcome, 0),
            "state=partial\nframes=0\ncached=no\nelapsed_ms=17\n"
        );
    }
}
