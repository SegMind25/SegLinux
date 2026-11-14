//! Links with linker.ld and generates the 256 interrupt entry stubs.
use std::fmt::Write;

/// Exceptions for which the CPU pushes an error code itself.
const HAS_ERROR_CODE: [u32; 10] = [8, 10, 11, 12, 13, 14, 17, 21, 29, 30];

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("TARGET").unwrap() == "x86_64-unknown-none" {
        println!("cargo:rustc-link-arg-bins=-T{dir}/linker.ld");
    }

    let mut s = String::from(".section .text\n");
    for v in 0..256u32 {
        writeln!(s, ".global isr_{v}\nisr_{v}:").unwrap();
        if !HAS_ERROR_CODE.contains(&v) {
            s.push_str("    push 0\n");
        }
        writeln!(s, "    push {v}\n    jmp isr_common").unwrap();
    }
    s.push_str(".section .rodata\n.balign 8\n.global isr_table\nisr_table:\n");
    for v in 0..256u32 {
        writeln!(s, "    .quad isr_{v}").unwrap();
    }
    let out = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{out}/isr_stubs.s"), s).unwrap();
}
