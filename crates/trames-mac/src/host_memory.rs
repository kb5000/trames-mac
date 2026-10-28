// SPDX-License-Identifier: GPL-3.0-or-later
//! Mac memory metadata only; no reservation, byte access, polling or hard quota.
use trames_linux::HostMemoryInfo;
// SDK Mach ABI. libc deprecates its convenience accessors, not these OS APIs.
unsafe extern "C" {
    fn mach_host_self() -> libc::mach_port_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
    static mach_task_self_: libc::mach_port_t;
}

pub fn physical_bytes() -> std::io::Result<usize> {
    let mut bytes = 0u64;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: exact initialized output/length for the hw.memsize scalar.
    if unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if len != std::mem::size_of::<u64>() || bytes == 0 {
        return Err(std::io::Error::other("invalid host physical memory size"));
    }
    usize::try_from(bytes).map_err(std::io::Error::other)
}

pub fn snapshot(total: usize) -> HostMemoryInfo {
    // A failed metadata query is conservative; it cannot impose an allocation
    // limit. The physical size was validated once when creating the session.
    let mut result = HostMemoryInfo {
        total,
        free: 0,
        available: 0,
    };
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: local complete zeroed structure, SDK-sized output and temporary
    // host send right, deallocated on both success and failure. No guest pointer.
    let status = unsafe {
        let host = mach_host_self();
        let status = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&mut stats as *mut libc::vm_statistics64).cast(),
            &mut count,
        );
        mach_port_deallocate(mach_task_self_, host);
        status
    };
    if status == libc::KERN_SUCCESS {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as usize;
        result.free = (stats.free_count as usize).saturating_mul(page).min(total);
        // SDK vm_statistics.h: speculative pages are already in free_count.
        // Purgeable pages can overlap the queues too. Count neither twice.
        // Free + inactive is an availability estimate, not Linux's full heuristic.
        result.available = (stats.free_count as usize)
            .saturating_add(stats.inactive_count as usize)
            .saturating_mul(page)
            .min(total);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_host_memory_is_bounded_and_not_a_fixed_guest_pool() {
        let total = super::physical_bytes().unwrap();
        let sample = super::snapshot(total);
        assert!(total > 0);
        assert_eq!(sample.total, total);
        assert!(sample.free <= sample.available && sample.available <= total);
    }
}
