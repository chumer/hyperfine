use std::io;
use std::mem::MaybeUninit;
use std::process::Child;

use crate::benchmark::measurement::HardwareCounters;
use crate::quantity::{byte, Information};

#[derive(Default)]
pub struct ProcessMetrics {
    pub hardware_counters: HardwareCounters,
    pub memory_peak_footprint: Option<Information>,
}

/// Wait for exit without reaping: proc_pid_rusage can still inspect the zombie,
/// and the existing wait4 call can subsequently collect CPU times and peak RSS.
pub fn wait(child: &mut Child) -> io::Result<()> {
    drop(child.stdin.take());
    let mut info = MaybeUninit::uninit();
    loop {
        // SAFETY: info points to writable siginfo_t storage. WNOWAIT preserves the child.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

pub fn read(pid: u32) -> ProcessMetrics {
    let mut usage = MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: the buffer has the layout and size requested by RUSAGE_INFO_V4.
    // Despite the pointer-to-pointer signature, this API writes directly into the buffer.
    let result = unsafe {
        libc::proc_pid_rusage(
            pid as libc::pid_t,
            libc::RUSAGE_INFO_V4,
            usage.as_mut_ptr().cast(),
        )
    };
    if result != 0 {
        return ProcessMetrics::default();
    }
    // SAFETY: the call succeeded and initialized the buffer.
    let usage = unsafe { usage.assume_init() };
    ProcessMetrics {
        hardware_counters: HardwareCounters {
            // Like macOS time(1), omit zero counters: the API can succeed on systems
            // where instruction/cycle accounting is unavailable.
            cpu_cycles: (usage.ri_cycles != 0).then_some(usage.ri_cycles),
            instructions: (usage.ri_instructions != 0).then_some(usage.ri_instructions),
            ..Default::default()
        },
        // The lifetime maximum survives exit and freeing memory. Unlike counters,
        // a successful query returning zero is still a valid memory measurement.
        memory_peak_footprint: Some(Information::new::<byte>(
            usage.ri_lifetime_max_phys_footprint as f64,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::unix_timer::CPUTimer;
    use std::process::Command;

    #[test]
    fn failed_query_leaves_missing_metrics_and_child_can_be_reaped() {
        let timer = CPUTimer::start();
        let mut child = Command::new("sh").args(["-c", "exit 7"]).spawn().unwrap();
        wait(&mut child).unwrap();
        let pid = child.id();
        let metrics = read(u32::MAX);
        assert!(metrics.memory_peak_footprint.is_none());
        assert_eq!(metrics.hardware_counters, HardwareCounters::default());
        assert!(read(pid).memory_peak_footprint.is_some());
        let (_, _, _, status) = timer.stop(child).unwrap();
        assert_eq!(status.code(), Some(7));
        assert!(read(pid).memory_peak_footprint.is_none());
    }
}
