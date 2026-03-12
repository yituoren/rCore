//! Process management syscalls
use crate::config::PAGE_SIZE;
use crate::mm::{PTEFlags, PageTable, VirtAddr};
use crate::task::{
    change_program_brk, current_mmap, current_munmap, current_user_token,
    exit_current_and_run_next, suspend_current_and_run_next,
};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let buffers = crate::mm::translated_byte_buffer(
        current_user_token(),
        ts as *const u8,
        core::mem::size_of::<TimeVal>(),
    );
    let src =
        unsafe { core::slice::from_raw_parts(&time_val as *const TimeVal as *const u8, core::mem::size_of::<TimeVal>()) };
    let mut offset = 0;
    for buf in buffers {
        let len = buf.len();
        buf.copy_from_slice(&src[offset..offset + len]);
        offset += len;
    }
    0
}

/// Check if a user virtual address is accessible with the given permission.
fn check_user_va(token: usize, va: usize, writable: bool) -> bool {
    let page_table = PageTable::from_token(token);
    let vpn = VirtAddr::from(va).floor();
    if let Some(pte) = page_table.translate(vpn) {
        if !pte.is_valid() {
            return false;
        }
        let flags = pte.flags();
        if (flags & PTEFlags::U) == PTEFlags::empty() {
            return false;
        }
        if writable && (flags & PTEFlags::W) == PTEFlags::empty() {
            return false;
        }
        if !writable && (flags & PTEFlags::R) == PTEFlags::empty() {
            return false;
        }
        true
    } else {
        false
    }
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    match trace_request {
        0 => {
            if !check_user_va(token, id, false) {
                return -1;
            }
            let page_table = PageTable::from_token(token);
            let vpn = VirtAddr::from(id).floor();
            let ppn = page_table.translate(vpn).unwrap().ppn();
            let offset = VirtAddr::from(id).page_offset();
            let pa = ppn.get_bytes_array();
            pa[offset] as isize
        }
        1 => {
            if !check_user_va(token, id, true) {
                return -1;
            }
            let page_table = PageTable::from_token(token);
            let vpn = VirtAddr::from(id).floor();
            let ppn = page_table.translate(vpn).unwrap().ppn();
            let offset = VirtAddr::from(id).page_offset();
            let pa = ppn.get_bytes_array();
            pa[offset] = data as u8;
            0
        }
        2 => crate::task::get_current_syscall_count(id) as isize,
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel: sys_mmap");
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    if port & !0x7 != 0 || port & 0x7 == 0 {
        return -1;
    }
    current_mmap(start, len, port)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    if start % PAGE_SIZE != 0 || (start + len) % PAGE_SIZE != 0 {
        return -1;
    }
    current_munmap(start, len)
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
