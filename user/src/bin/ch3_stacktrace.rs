#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use core::arch::asm;

#[inline(never)]
fn print_stackframe() {
    let mut fp: usize;
    unsafe {
        asm!("mv {}, s0", out(reg) fp);
    }
    let mut i = 0;
    while fp != 0 && fp % 8 == 0 && fp >= 0x10000 {
        // 返回地址在 fp - 8，上一个栈帧的 fp 在 fp - 16
        let ra = unsafe { *((fp - 8) as *const usize) };
        let prev_fp = unsafe { *((fp - 16) as *const usize) };
        println!("#{}: ra = 0x{:x}", i, ra);
        i += 1;
        fp = prev_fp;
        if i >= 20 {
            break;
        }
    }
}

#[inline(never)]
fn func_c() {
    // 头调用和尾调用可能会被编译器优化掉，导致栈帧缺失，因此使用 #[inline(never)] 且加入 println! 防止优化
    println!("entering func_c");
    print_stackframe();
    println!("leaving func_c");
}

#[inline(never)]
fn func_b() {
    println!("entering func_b");
    func_c();
    println!("leaving func_b");
}

#[inline(never)]
fn func_a() {
    println!("entering func_a");
    func_b();
    println!("leaving func_a");
}

#[no_mangle]
fn main() -> i32 {
    println!("== test_stacktrace ==");
    func_a();
    println!("== stacktrace done ==");
    0
}
