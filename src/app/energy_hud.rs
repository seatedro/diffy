use std::collections::HashMap;
use std::time::{Duration, Instant};

use quark::view;
use quark_app::ViewContext;
use quark_ui::element::*;
use quark_ui::style::Styled;

use crate::app::tokens::{self, Sp, TextStyle};
use crate::platform::energy::{self, Snapshot};

const REFRESH: Duration = Duration::from_secs(1);
const TOP_THREADS: usize = 6;

#[derive(Debug, Default)]
pub struct EnergyHud {
    last: Option<(Instant, Snapshot, HashMap<String, u64>)>,
    frames: u32,
    lines: Vec<String>,
}

fn thread_group(name: &str) -> String {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-' || c == '_');
    if trimmed.is_empty() {
        "unnamed".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn grouped_threads() -> HashMap<String, u64> {
    let mut groups = HashMap::new();
    for (name, cpu) in energy::threads() {
        *groups.entry(thread_group(&name)).or_insert(0) += cpu;
    }
    groups
}

impl EnergyHud {
    pub fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        self.frames += 1;
        let now = Instant::now();
        let due = self
            .last
            .as_ref()
            .is_none_or(|(at, _, _)| now.duration_since(*at) >= REFRESH);
        if due {
            self.refresh(now, cx.frame.last_render_stats().cpu_us);
        }
        cx.frame.request_frame_in(REFRESH);
        let theme = cx.theme;
        let tc = &theme.colors;
        let scale = tokens::ui_scale(theme);
        let mono = TextStyle::MonoSmall.font_size(theme);
        let rows: Vec<AnyElement> = self
            .lines
            .iter()
            .map(|line| view! { <text size={mono} class="font-mono" color={tc.text}>{line.clone()}</text> })
            .collect();
        view! {
            <div
                class="absolute flex-col"
                right={tokens::scaled(Sp::MD, scale)}
                bottom={tokens::scaled(Sp::XXL, scale)}
                p={tokens::scaled(Sp::SM, scale)}
                gap={tokens::scaled(Sp::XXS, scale)}
                bg={tc.elevated_surface}
                border={tc.border}
                rounded={theme.metrics.panel_radius}
                z_index={900}
                aria-label="Energy"
                test_id="energy.hud"
            >
                {...rows}
            </div>
        }
    }

    fn refresh(&mut self, now: Instant, render_us: u64) {
        let snapshot = energy::snapshot();
        let threads = grouped_threads();
        let Some((at, previous, previous_threads)) = self.last.take() else {
            self.last = Some((now, snapshot, threads));
            self.frames = 0;
            self.lines = vec!["Energy: measuring…".to_owned()];
            return;
        };
        let secs = now.duration_since(at).as_secs_f64().max(0.001);
        let delta = snapshot.since(&previous);
        let process = delta.process;
        let mut lines = vec![
            format!(
                "cpu {:5.1}%  wakeups {:4.0}/s  power {:5.0} mW",
                process.cpu_ns as f64 / (secs * 1e9) * 100.0,
                process.idle_wakeups as f64 / secs,
                process.energy_nj as f64 / secs / 1e6,
            ),
            format!(
                "frames {:4.1}/s  render {:5.2} ms  git {:3.0}/s {:5.1}%",
                f64::from(self.frames) / secs,
                render_us as f64 / 1000.0,
                delta.processes as f64 / secs,
                process.child_cpu_ns as f64 / (secs * 1e9) * 100.0,
            ),
        ];
        let work: Vec<String> = delta
            .busiest()
            .iter()
            .map(|(kind, cpu, _)| {
                format!("{} {:.0}%", kind.label(), cpu.as_secs_f64() / secs * 100.0)
            })
            .collect();
        if !work.is_empty() {
            lines.push(format!("work {}", work.join("  ")));
        }
        let mut busiest: Vec<(String, f64)> = threads
            .iter()
            .map(|(name, cpu)| {
                let before = previous_threads.get(name).copied().unwrap_or(0);
                (
                    name.clone(),
                    cpu.saturating_sub(before) as f64 / (secs * 1e9) * 100.0,
                )
            })
            .filter(|(_, pct)| *pct >= 0.5)
            .collect();
        busiest.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (name, pct) in busiest.into_iter().take(TOP_THREADS) {
            lines.push(format!("  {pct:5.1}%  {name}"));
        }
        self.lines = lines;
        self.frames = 0;
        self.last = Some((now, snapshot, threads));
    }
}

#[cfg(test)]
mod tests {
    use super::thread_group;

    #[test]
    fn worker_threads_group_by_name_without_their_number() {
        assert_eq!(thread_group("diffy-semantic-3"), "diffy-semantic");
        assert_eq!(thread_group("tokio-runtime-worker"), "tokio-runtime-worker");
        assert_eq!(thread_group(""), "unnamed");
        assert_eq!(thread_group("main"), "main");
    }
}
