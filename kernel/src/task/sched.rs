//! The scheduler: round-robin over one shared run queue, preemptive.
//!
//! Each CPU runs its current task until it blocks, exits, yields, or the
//! timer interrupt preempts it (every tick, 1/HZ s). A per-CPU idle task
//! runs when the queue is empty. Per-CPU queues with work stealing and
//! priorities come when there is load to measure them with.
//!
//! Switching: `switch_context` saves the callee-saved registers on the old
//! stack, stores its stack pointer, loads the new one and returns into the
//! new task. It hands the previous task to the new side, which marks it
//! off-CPU (`finish_switch`); only then may another CPU run it.

use super::{State, Task};
use crate::hal::percpu::{self, this_cpu};
use crate::sync::SpinLock;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use x86_64::instructions::interrupts;

static RUN_QUEUE: SpinLock<VecDeque<Arc<Task>>> = SpinLock::new(VecDeque::new());

core::arch::global_asm!(
    ".section .text",
    // rdi: where to save the old rsp, rsi: new rsp, rdx: previous task
    ".global switch_context",
    "switch_context:",
    "    push rbp",
    "    push rbx",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    "    mov [rdi], rsp",
    "    mov rsp, rsi",
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop rbx",
    "    pop rbp",
    "    mov rax, rdx",
    "    ret",
    // First run of a new task: rax holds the previous task.
    ".global task_trampoline",
    "task_trampoline:",
    "    mov rdi, rax",
    "    and rsp, -16",
    "    call task_start",
    "    ud2",
);

unsafe extern "C" {
    fn switch_context(save: *mut u64, load: u64, prev: *const ()) -> *const ();
    fn task_trampoline();
}

/// The stack of a task that has never run: six zeroed callee-saved
/// registers (rbp = 0 ends backtraces) and a return into the trampoline.
pub(super) fn initial_stack(top: u64) -> u64 {
    let rsp = top - 8 * 8;
    unsafe {
        let slots = rsp as *mut u64;
        for i in 0..6 {
            slots.add(i).write(0);
        }
        slots.add(6).write(task_trampoline as *const () as u64);
        slots.add(7).write(0);
    }
    rsp
}

/// Makes the code running on this CPU its idle task. Call once per CPU,
/// interrupts disabled.
pub fn init_cpu() {
    let cpu = this_cpu();
    let idle = Arc::new(Task::boot("idle"));
    idle.last_cpu.store(cpu.index, Ordering::Relaxed);
    cpu.idle.call_once(|| idle.clone());
    unsafe { *cpu.current.get() = Some(idle) };
    if cpu.index == 0 {
        RUN_QUEUE.lock().reserve(64);
    }
}

/// Runs the idle loop forever: sleep until an interrupt, then look for work.
pub fn idle_loop() -> ! {
    loop {
        interrupts::enable_and_hlt();
        schedule();
    }
}

pub fn spawn(f: impl FnOnce() + Send + 'static) -> Arc<Task> {
    super::spawn_named("kthread", f)
}

pub(super) fn spawn_task(task: Task) -> Arc<Task> {
    let task = Arc::new(task);
    RUN_QUEUE.lock().push_back(task.clone());
    task
}

pub fn current() -> Arc<Task> {
    interrupts::without_interrupts(|| unsafe {
        (*this_cpu().current.get())
            .clone()
            .expect("scheduler not running on this CPU")
    })
}

/// Puts a blocked or new task on the run queue.
pub(crate) fn make_ready(task: Arc<Task>) {
    RUN_QUEUE.lock().push_back(task);
}

/// Called from the timer interrupt.
pub fn preempt() {
    if unsafe { (*this_cpu().current.get()).is_some() } {
        schedule();
    }
}

/// Gives the CPU to the next ready task (or keeps the current one if
/// nothing else is ready and it can still run).
pub fn schedule() {
    let irq = interrupts::are_enabled();
    interrupts::disable();
    let cpu = this_cpu();
    let slot = unsafe { &mut *cpu.current.get() };
    let cur = slot.as_ref().expect("scheduler not running on this CPU");
    let idle = cpu.idle.get().unwrap();
    let cur_is_idle = Arc::ptr_eq(cur, idle);

    let next = {
        let mut q = RUN_QUEUE.lock();
        if !cur_is_idle && cur.cas_state(State::Running, State::Ready) {
            q.push_back(cur.clone());
        }
        match q.pop_front() {
            Some(t) => t,
            None if cur_is_idle => idle.clone(),
            // A blocked or dead task cannot continue; a woken one (Ready,
            // already queued) would have been popped above.
            None => idle.clone(),
        }
    };

    if Arc::ptr_eq(&next, cur) {
        if !cur_is_idle {
            cur.set_state(State::Running);
        }
        drop(next);
        if irq {
            interrupts::enable();
        }
        return;
    }

    // The task may still be finishing its switch away on another CPU.
    while next.on_cpu.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    next.on_cpu.store(true, Ordering::Relaxed);
    if !Arc::ptr_eq(&next, idle) {
        next.set_state(State::Running);
    }
    next.last_cpu.store(cpu.index, Ordering::Relaxed);
    let next_rsp = unsafe { *next.rsp.get() };

    let prev = slot.replace(next).unwrap();
    let save = prev.rsp.get();
    // No owned Arc may live across the switch: a dead task never comes back
    // to drop it.
    let prev = Arc::into_raw(prev);
    let prev = unsafe { switch_context(save, next_rsp, prev.cast()) }.cast::<Task>();
    finish_switch(prev);
    if irq {
        interrupts::enable();
    }
}

/// First thing on the new side of a switch.
fn finish_switch(prev: *const Task) {
    let prev = unsafe { Arc::from_raw(prev) };
    prev.on_cpu.store(false, Ordering::Release);
    // Dropping the last reference to a dead task frees its stack: safe, we
    // are on another stack now.
}

#[unsafe(no_mangle)]
extern "C" fn task_start(prev: *const Task) -> ! {
    finish_switch(prev);
    let entry = {
        let me = current();
        unsafe { (*me.entry.get()).take() }
    };
    interrupts::enable();
    if let Some(f) = entry {
        f();
    }
    exit()
}

/// Ends the current task.
pub fn exit() -> ! {
    interrupts::disable();
    {
        let me = unsafe { (*this_cpu().current.get()).as_ref().unwrap() };
        me.set_state(State::Dead);
        me.exited.wake_all();
    }
    schedule();
    unreachable!("a dead task was scheduled again");
}

pub fn online_cpus() -> usize {
    percpu::count()
}
