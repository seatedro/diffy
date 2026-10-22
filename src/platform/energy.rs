use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SUMMARY_EVERY: Duration = Duration::from_secs(300);
const LOG_JOB_CPU: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Compare,
    Semantic,
    LineStats,
    Highlight,
    Index,
    Resolve,
    Files,
    Render,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Compare,
        Kind::Semantic,
        Kind::LineStats,
        Kind::Highlight,
        Kind::Index,
        Kind::Resolve,
        Kind::Files,
        Kind::Render,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Compare => "compare",
            Kind::Semantic => "semantic",
            Kind::LineStats => "line-stats",
            Kind::Highlight => "highlight",
            Kind::Index => "index",
            Kind::Resolve => "resolve",
            Kind::Files => "files",
            Kind::Render => "render",
        }
    }
}

struct Counter {
    cpu_ns: AtomicU64,
    jobs: AtomicU64,
}

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: Counter = Counter {
    cpu_ns: AtomicU64::new(0),
    jobs: AtomicU64::new(0),
};

static COUNTERS: [Counter; Kind::ALL.len()] = [ZERO; Kind::ALL.len()];
static PROCESSES: AtomicU64 = AtomicU64::new(0);
static SUMMARY: Mutex<Option<(Instant, Snapshot)>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessUsage {
    pub cpu_ns: u64,
    pub child_cpu_ns: u64,
    pub idle_wakeups: u64,
    pub interrupt_wakeups: u64,
    pub energy_nj: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub process: ProcessUsage,
    pub work: [(u64, u64); Kind::ALL.len()],
    pub processes: u64,
}

impl Snapshot {
    pub fn since(&self, earlier: &Snapshot) -> Snapshot {
        let mut work = [(0, 0); Kind::ALL.len()];
        for (slot, (now, then)) in work.iter_mut().zip(self.work.iter().zip(&earlier.work)) {
            *slot = (now.0.saturating_sub(then.0), now.1.saturating_sub(then.1));
        }
        let (a, b) = (&self.process, &earlier.process);
        Snapshot {
            process: ProcessUsage {
                cpu_ns: a.cpu_ns.saturating_sub(b.cpu_ns),
                child_cpu_ns: a.child_cpu_ns.saturating_sub(b.child_cpu_ns),
                idle_wakeups: a.idle_wakeups.saturating_sub(b.idle_wakeups),
                interrupt_wakeups: a.interrupt_wakeups.saturating_sub(b.interrupt_wakeups),
                energy_nj: a.energy_nj.saturating_sub(b.energy_nj),
            },
            work,
            processes: self.processes.saturating_sub(earlier.processes),
        }
    }

    pub fn busiest(&self) -> Vec<(Kind, Duration, u64)> {
        let mut busiest: Vec<(Kind, Duration, u64)> = Kind::ALL
            .iter()
            .zip(&self.work)
            .filter(|(_, (cpu, _))| *cpu > 0)
            .map(|(kind, (cpu, jobs))| (*kind, Duration::from_nanos(*cpu), *jobs))
            .collect();
        busiest.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        busiest
    }
}

pub fn snapshot() -> Snapshot {
    let mut work = [(0, 0); Kind::ALL.len()];
    for (slot, counter) in work.iter_mut().zip(&COUNTERS) {
        *slot = (
            counter.cpu_ns.load(Ordering::Relaxed),
            counter.jobs.load(Ordering::Relaxed),
        );
    }
    Snapshot {
        process: process(),
        work,
        processes: PROCESSES.load(Ordering::Relaxed),
    }
}

pub fn count_process() {
    PROCESSES.fetch_add(1, Ordering::Relaxed);
}

pub struct Work {
    kind: Kind,
    cpu_ns: u64,
    started: Instant,
    detail: Option<String>,
}

pub fn work(kind: Kind) -> Work {
    Work {
        kind,
        cpu_ns: thread_cpu_ns(),
        started: Instant::now(),
        detail: None,
    }
}

impl Work {
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let cpu = thread_cpu_ns().saturating_sub(self.cpu_ns);
        let counter = &COUNTERS[self.kind as usize];
        counter.cpu_ns.fetch_add(cpu, Ordering::Relaxed);
        counter.jobs.fetch_add(1, Ordering::Relaxed);
        let cpu = Duration::from_nanos(cpu);
        if cpu >= LOG_JOB_CPU {
            tracing::info!(
                target: "diffy::energy",
                kind = self.kind.label(),
                cpu_ms = cpu.as_millis() as u64,
                wall_ms = self.started.elapsed().as_millis() as u64,
                detail = self.detail.as_deref().unwrap_or(""),
                "work"
            );
        }
        log_summary_if_due();
    }
}

fn log_summary_if_due() {
    let Ok(mut summary) = SUMMARY.try_lock() else {
        return;
    };
    let now = Instant::now();
    let Some((at, previous)) = summary.as_ref() else {
        *summary = Some((now, snapshot()));
        return;
    };
    let elapsed = now.duration_since(*at);
    if elapsed < SUMMARY_EVERY {
        return;
    }
    let current = snapshot();
    let delta = current.since(previous);
    let busiest: Vec<String> = delta
        .busiest()
        .iter()
        .map(|(kind, cpu, jobs)| format!("{}={}ms/{jobs}", kind.label(), cpu.as_millis()))
        .collect();
    tracing::info!(
        target: "diffy::energy",
        secs = elapsed.as_secs(),
        cpu_ms = delta.process.cpu_ns / 1_000_000,
        cpu_pct = delta.process.cpu_ns as f64 / elapsed.as_nanos() as f64 * 100.0,
        child_cpu_ms = delta.process.child_cpu_ns / 1_000_000,
        energy_mj = delta.process.energy_nj / 1_000_000,
        idle_wakeups = delta.process.idle_wakeups,
        processes = delta.processes,
        work = busiest.join(" "),
        "summary"
    );
    *summary = Some((now, current));
}

pub fn background_thread() {
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

pub fn thread_cpu_ns() -> u64 {
    #[cfg(unix)]
    {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } == 0 {
            return time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64;
        }
        0
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
    static mach_task_self_: libc::mach_port_t;
}

#[cfg(target_os = "macos")]
fn process() -> ProcessUsage {
    use std::sync::OnceLock;

    static TIMEBASE: OnceLock<(u64, u64)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        unsafe { mach_timebase_info(&mut info) };
        (u64::from(info.numer.max(1)), u64::from(info.denom.max(1)))
    });
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    let status = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as libc::c_int,
            libc::RUSAGE_INFO_V4,
            usage.as_mut_ptr().cast(),
        )
    };
    if status != 0 {
        return ProcessUsage::default();
    }
    let usage = unsafe { usage.assume_init() };
    let ns = |ticks: u64| (u128::from(ticks) * u128::from(numer) / u128::from(denom)) as u64;
    ProcessUsage {
        cpu_ns: ns(usage.ri_user_time + usage.ri_system_time),
        child_cpu_ns: ns(usage.ri_child_user_time + usage.ri_child_system_time),
        idle_wakeups: usage.ri_pkg_idle_wkups,
        interrupt_wakeups: usage.ri_interrupt_wkups,
        energy_nj: usage.ri_billed_energy,
    }
}

#[cfg(target_os = "macos")]
pub fn threads() -> Vec<(String, u64)> {
    unsafe extern "C" {
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
    }
    let task = unsafe { mach_task_self_ };
    let mut list: libc::thread_act_array_t = std::ptr::null_mut();
    let mut count: libc::mach_msg_type_number_t = 0;
    if unsafe { libc::task_threads(task, &mut list, &mut count) } != 0 || list.is_null() {
        return Vec::new();
    }
    let mut threads = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let thread = unsafe { *list.add(index) };
        let mut info = std::mem::MaybeUninit::<libc::thread_extended_info>::zeroed();
        let mut size = libc::THREAD_EXTENDED_INFO_COUNT;
        let status = unsafe {
            libc::thread_info(
                thread,
                libc::THREAD_EXTENDED_INFO as libc::thread_flavor_t,
                info.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status == 0 {
            let info = unsafe { info.assume_init() };
            let name: Vec<u8> = info
                .pth_name
                .iter()
                .take_while(|byte| **byte != 0)
                .map(|byte| *byte as u8)
                .collect();
            threads.push((
                String::from_utf8_lossy(&name).into_owned(),
                info.pth_user_time + info.pth_system_time,
            ));
        }
        unsafe { mach_port_deallocate(task, thread) };
    }
    unsafe {
        libc::vm_deallocate(
            task,
            list as libc::vm_address_t,
            count as usize * std::mem::size_of::<libc::thread_act_t>(),
        )
    };
    threads
}

#[cfg(not(target_os = "macos"))]
pub fn threads() -> Vec<(String, u64)> {
    Vec::new()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn process() -> ProcessUsage {
    let cpu = |who| {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        if unsafe { libc::getrusage(who, usage.as_mut_ptr()) } != 0 {
            return 0;
        }
        let usage = unsafe { usage.assume_init() };
        let ns = |t: libc::timeval| t.tv_sec as u64 * 1_000_000_000 + t.tv_usec as u64 * 1_000;
        ns(usage.ru_utime) + ns(usage.ru_stime)
    };
    ProcessUsage {
        cpu_ns: cpu(libc::RUSAGE_SELF),
        child_cpu_ns: cpu(libc::RUSAGE_CHILDREN),
        ..ProcessUsage::default()
    }
}

#[cfg(not(unix))]
fn process() -> ProcessUsage {
    ProcessUsage::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_charges_thread_cpu_to_its_kind() {
        let before = snapshot();
        {
            let _work = work(Kind::Resolve);
            let started = Instant::now();
            let mut spin = 0u64;
            while started.elapsed() < Duration::from_millis(20) {
                spin = spin.wrapping_add(1);
            }
            std::hint::black_box(spin);
        }
        let delta = snapshot().since(&before);
        let (cpu, jobs) = delta.work[Kind::Resolve as usize];
        assert!(jobs >= 1);
        assert!(cpu >= 10_000_000, "charged {cpu}ns");
        assert!(delta.process.cpu_ns > 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn threads_report_names_and_cpu() {
        let handle = std::thread::Builder::new()
            .name("energy-probe".to_owned())
            .spawn(|| {
                let started = Instant::now();
                while started.elapsed() < Duration::from_millis(30) {}
                threads()
            })
            .unwrap();
        let threads = handle.join().unwrap();
        let probe = threads.iter().find(|(name, _)| name == "energy-probe");
        assert!(
            probe.is_some_and(|(_, cpu)| *cpu > 10_000_000),
            "{threads:?}"
        );
    }
}
