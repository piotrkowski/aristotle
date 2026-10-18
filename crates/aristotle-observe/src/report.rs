// SPDX-License-Identifier: AGPL-3.0-or-later
//! Release-based measurements from touch and driver timestamps.

use crate::kmsg::Update;
use crate::{Interaction, SETTLE_US};

#[derive(Debug, serde::Serialize)]
pub struct Measurement<'a> {
    pub interaction: &'a Interaction,
    pub updates: Vec<&'a Update>,
    pub pre_release: usize,
    pub incomplete: usize,
    pub valid: bool,
    pub first_start_us: Option<u64>,
    pub first_done_us: Option<u64>,
    pub page_done_us: Option<u64>,
    pub settled_us: Option<u64>,
}

pub fn measure<'a>(
    interactions: &'a [Interaction],
    updates: &'a [Update],
    lost_records: u64,
) -> Vec<Measurement<'a>> {
    let mut ordered: Vec<_> = updates.iter().collect();
    ordered.sort_by_key(|u| u.start_us);
    interactions
        .iter()
        .enumerate()
        .map(|(index, interaction)| {
            let next = interactions.get(index + 1).map_or(u64::MAX, |i| i.down_us);
            let mut deadline = interaction.up_us.saturating_add(SETTLE_US);
            let mut mine = Vec::new();
            for update in ordered
                .iter()
                .copied()
                .filter(|u| u.start_us >= interaction.down_us && u.start_us < next)
            {
                if update.start_us >= deadline {
                    break;
                }
                deadline = deadline.max(
                    update
                        .end_us
                        .unwrap_or(update.start_us)
                        .saturating_add(SETTLE_US),
                );
                mine.push(update);
            }
            let pre_release = mine
                .iter()
                .filter(|u| u.start_us < interaction.up_us)
                .count();
            let incomplete = mine.iter().filter(|u| u.end_us.is_none()).count();
            let after_release = || {
                mine.iter()
                    .copied()
                    .filter(|u| u.start_us >= interaction.up_us)
            };
            let first_start_us = after_release()
                .map(|u| u.start_us - interaction.up_us)
                .min();
            let valid = incomplete == 0 && lost_records == 0;
            let completion = |updates: Vec<&Update>| {
                valid
                    .then(|| updates.iter().filter_map(|u| u.end_us).min())
                    .flatten()
                    .and_then(|end| end.checked_sub(interaction.up_us))
            };
            let first_done_us = completion(after_release().collect());
            let page_done_us = completion(
                mine.iter()
                    .copied()
                    .filter(|u| u.end_us.is_some_and(|end| end >= interaction.up_us))
                    .filter(|u| {
                        let (top, left, width, height) = u.rect;
                        top == 0 && left == 0 && width >= 1072 && height >= 1448
                    })
                    .collect(),
            );
            let settled_us = valid
                .then(|| mine.iter().filter_map(|u| u.end_us).max())
                .flatten()
                .and_then(|end| end.checked_sub(interaction.up_us));
            Measurement {
                interaction,
                updates: mine,
                pre_release,
                incomplete,
                valid,
                first_start_us,
                first_done_us,
                page_done_us,
                settled_us,
            }
        })
        .collect()
}

pub fn median(values: impl Iterator<Item = u64>) -> Option<f64> {
    let mut values: Vec<_> = values.collect();
    values.sort_unstable();
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 1 => Some(values[middle] as f64),
        _ => Some(values[middle - 1] as f64 / 2.0 + values[middle] as f64 / 2.0),
    }
}

fn time(value: Option<u64>) -> String {
    value.map_or_else(|| "n/a".into(), |us| format!("{:.1}ms", us as f64 / 1000.0))
}

pub fn print(interactions: &[Interaction], updates: &[Update], lost_records: u64) {
    if std::env::var_os("OBSERVE_DEBUG").is_some() {
        for update in updates {
            eprintln!("update: {update:?}");
        }
        for interaction in interactions {
            eprintln!("touch: {interaction:?}");
        }
    }
    let rows = measure(interactions, updates, lost_records);
    println!(
        "{:<22} {:>10} {:>10} {:>10} {:>10} {:>4} {:>4} {:>4} {:>5} {:>7}  waveforms",
        "interaction",
        "first>",
        "first✓",
        "page✓",
        "settled",
        "upd",
        "pre",
        "open",
        "flash",
        "area%"
    );
    for row in &rows {
        let i = row.interaction;
        let label = if i.label.is_empty() {
            format!("({},{})", i.x, i.y)
        } else {
            i.label.clone()
        };
        let flashes = row.updates.iter().filter(|u| u.flashes()).count();
        let area: u64 = row.updates.iter().map(|u| u.area()).sum();
        let mut waveforms: Vec<_> = row.updates.iter().map(|u| u.waveform.as_str()).collect();
        waveforms.dedup();
        println!(
            "{label:<22} {:>10} {:>10} {:>10} {:>10} {:>4} {:>4} {:>4} {:>5} {:>6.0}%  {}",
            time(row.first_start_us),
            time(row.first_done_us),
            time(row.page_done_us),
            time(row.settled_us),
            row.updates.len(),
            row.pre_release,
            row.incomplete,
            flashes,
            area as f64 / (1072.0 * 1448.0) * 100.0,
            waveforms.join(",")
        );
    }
    for (name, values) in [
        (
            "first✓",
            rows.iter()
                .filter_map(|r| r.first_done_us)
                .collect::<Vec<_>>(),
        ),
        (
            "page✓",
            rows.iter().filter_map(|r| r.page_done_us).collect(),
        ),
        (
            "settled",
            rows.iter().filter_map(|r| r.settled_us).collect(),
        ),
    ] {
        let count = values.len();
        if let Some(us) = median(values.into_iter()) {
            println!(
                "summary: {name} median {:.1} ms, {count}/{} samples",
                us / 1000.0,
                rows.len()
            );
        }
    }
    if lost_records > 0 {
        println!("invalid capture: {lost_records} kernel log overruns; completion metrics omitted");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(down_us: u64, up_us: u64) -> Interaction {
        Interaction {
            label: "turn".into(),
            down_us,
            up_us,
            x: 900,
            y: 800,
        }
    }

    fn update(start_us: u64, end_us: Option<u64>, page: bool) -> Update {
        Update {
            start_us,
            end_us,
            rect: if page {
                (0, 0, 1072, 1448)
            } else {
                (0, 0, 100, 100)
            },
            ..Default::default()
        }
    }

    #[test]
    fn separates_touch_down_acknowledgement_from_the_page() {
        let interactions = [touch(100, 200)];
        let updates = [update(110, Some(250), false), update(220, Some(800), true)];
        let rows = measure(&interactions, &updates, 0);
        assert_eq!(rows[0].pre_release, 1);
        assert_eq!(rows[0].first_start_us, Some(20));
        assert_eq!(rows[0].first_done_us, Some(600));
        assert_eq!(rows[0].page_done_us, Some(600));
    }

    #[test]
    fn uses_the_earliest_completion_not_the_first_submit() {
        let interactions = [touch(100, 200)];
        let updates = [update(220, Some(800), true), update(230, Some(400), false)];
        let rows = measure(&interactions, &updates, 0);
        assert_eq!(rows[0].first_done_us, Some(200));
        assert_eq!(rows[0].page_done_us, Some(600));
    }

    #[test]
    fn full_screen_updates_spanning_release_keep_their_actual_completion() {
        let interactions = [touch(100, 200)];
        let updates = [update(110, Some(180), true), update(190, Some(800), true)];
        let rows = measure(&interactions, &updates, 0);
        assert_eq!(rows[0].pre_release, 2);
        assert_eq!(rows[0].first_done_us, None);
        assert_eq!(rows[0].page_done_us, Some(600));
        assert_eq!(rows[0].settled_us, Some(600));
    }

    #[test]
    fn missing_completion_or_log_loss_invalidates_completion_metrics() {
        let interactions = [touch(100, 200)];
        let updates = [update(220, None, true), update(230, Some(400), false)];
        let rows = measure(&interactions, &updates, 0);
        assert_eq!(rows[0].incomplete, 1);
        assert_eq!(
            (
                rows[0].first_done_us,
                rows[0].page_done_us,
                rows[0].settled_us
            ),
            (None, None, None)
        );
        let complete = [update(220, Some(800), true)];
        let rows = measure(&interactions, &complete, 1);
        assert_eq!(rows[0].first_done_us, None);
    }

    #[test]
    fn extends_the_quiet_window_but_stops_at_the_next_touch() {
        let interactions = [touch(100, 200), touch(3_000_000, 3_000_100)];
        let updates = [
            update(220, Some(1_000_000), true),
            update(2_000_000, Some(2_500_000), false),
            update(3_000_050, Some(3_500_000), false),
        ];
        let rows = measure(&interactions, &updates, 0);
        assert_eq!(rows[0].updates.len(), 2);
        assert_eq!(rows[0].settled_us, Some(2_499_800));
        assert_eq!(rows[1].updates.len(), 1);
    }

    #[test]
    fn quiet_or_pre_release_only_interactions_have_no_release_sample() {
        let interactions = [touch(100, 200)];
        assert_eq!(measure(&interactions, &[], 0)[0].first_done_us, None);
        let updates = [update(110, Some(180), false)];
        let rows = measure(&interactions, &updates, 0);
        assert_eq!(rows[0].first_done_us, None);
        assert_eq!(rows[0].page_done_us, None);
    }

    #[test]
    fn median_averages_the_middle_pair() {
        assert_eq!(median([600, 100, 200, 300].into_iter()), Some(250.0));
        assert_eq!(median([600, 100, 200].into_iter()), Some(200.0));
        assert_eq!(median([].into_iter()), None);
    }
}
