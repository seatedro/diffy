use std::time::Instant;

pub const BUDGET_120_US: u64 = 8_333;
pub const BUDGET_60_US: u64 = 16_667;

const HISTORY: usize = 120;
const EMA_ALPHA: f32 = 0.1;
const RESYNC_GAP_US: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, Default)]
pub struct HudSample {
    pub build_us: u64,
    pub paint_us: u64,
    pub render_cpu_us: u64,
    pub acquire_us: u64,
    pub present_us: u64,
    pub primitive_count: usize,
    pub frame_interval_us: u64,
}

impl HudSample {
    pub fn cpu_us(&self) -> u64 {
        self.build_us + self.paint_us + self.render_cpu_us
    }
}

#[derive(Debug)]
pub struct HudState {
    pub last: HudSample,
    history: [u32; HISTORY],
    head: usize,
    filled: usize,
    last_frame_at: Option<Instant>,
    cpu_ema_us: f32,
    interval_ema_us: f32,
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            last: HudSample::default(),
            history: [0; HISTORY],
            head: 0,
            filled: 0,
            last_frame_at: None,
            cpu_ema_us: 0.0,
            interval_ema_us: 0.0,
        }
    }
}

impl HudState {
    pub fn frame_started(&mut self, now: Instant) -> u64 {
        let interval = self
            .last_frame_at
            .map(|prev| now.duration_since(prev).as_micros() as u64)
            .unwrap_or(0);
        self.last_frame_at = Some(now);
        if interval > 0 && interval < RESYNC_GAP_US {
            self.interval_ema_us = ema(self.interval_ema_us, interval as f32);
        }
        interval
    }

    pub fn record(&mut self, sample: HudSample) {
        self.last = sample;
        let cpu = sample.cpu_us();
        self.cpu_ema_us = ema(self.cpu_ema_us, cpu as f32);
        self.history[self.head] = cpu.min(u64::from(u32::MAX)) as u32;
        self.head = (self.head + 1) % HISTORY;
        self.filled = (self.filled + 1).min(HISTORY);
    }

    pub fn cpu_ema_us(&self) -> u64 {
        self.cpu_ema_us as u64
    }

    pub fn fps(&self) -> f32 {
        if self.interval_ema_us > 0.0 {
            1_000_000.0 / self.interval_ema_us
        } else {
            0.0
        }
    }

    pub fn samples(&self) -> impl Iterator<Item = u32> + '_ {
        let start = (self.head + HISTORY - self.filled) % HISTORY;
        (0..self.filled).map(move |i| self.history[(start + i) % HISTORY])
    }

    pub fn history_len(&self) -> usize {
        self.filled
    }

    pub fn history_capacity(&self) -> usize {
        HISTORY
    }

    pub fn history_peak_us(&self) -> u64 {
        self.samples().max().unwrap_or(0) as u64
    }
}

fn ema(prev: f32, next: f32) -> f32 {
    if prev <= 0.0 {
        next
    } else {
        prev + EMA_ALPHA * (next - prev)
    }
}
