//! Named FreeRTOS tasks, and what their stacks actually cost.
//!
//! Every Rust `std` thread on ESP-IDF is a pthread, and a pthread with no
//! explicit configuration becomes a FreeRTOS task called `pthread` with
//! `CONFIG_PTHREAD_TASK_STACK_SIZE_DEFAULT` bytes of stack. The name is
//! what FreeRTOS prints when a stack overflows, so a board that dies with
//! `Detected in task "pthread"` tells you nothing about which of a dozen
//! threads blew up - which is exactly what happened when `lto = true`
//! inlined a 8.5 KB constructor into the dispatcher's frame (2026-11-04).
//!
//! [`spawn`] fixes that: it sets the esp_pthread configuration for the
//! calling thread so the task gets a real name, spawns through
//! [`std::thread::Builder`] with the same name and stack size, and then
//! puts the default configuration back so a later plain `thread::spawn`
//! elsewhere is not silently handed this thread's name and stack.
//!
//! It also remembers `name -> stack size`, which FreeRTOS does not keep
//! anywhere readable on a downward-growing stack port. That registry is
//! what lets [`report`] say "24576 bytes, 38 % used" instead of only "the
//! least it ever had left was 15064 bytes".
//!
//! The tasks ESP-IDF creates for itself are covered too, as far as their
//! size is a build-time constant: `IDF_TASKS` and `IDLE_TASK` read it
//! out of `sdkconfig`, which costs nothing at runtime. That is how the
//! report caught `sys_evt` at 956 bytes free of 2304 (2026-11-04).
//!
//! Names are capped by `configMAX_TASK_NAME_LEN`
//! (`CONFIG_FREERTOS_MAX_TASK_NAME_LEN`, 16 including the NUL, so 15
//! characters); FreeRTOS truncates silently, so keep them short.
//!
//! Units: ESP-IDF's FreeRTOS reports the high-water mark in BYTES, not in
//! words as upstream FreeRTOS documents. `StackType_t` is `uint8_t` on
//! this port (`portSTACK_TYPE` in
//! `components/freertos/FreeRTOS-Kernel/portable/riscv/include/freertos/portmacro.h`)
//! and ESP-IDF's own doc comment on `uxTaskGetStackHighWaterMark` spells
//! it out: "the minimum free stack space there has been in bytes (as
//! opposed to words in the standard FreeRTOS documentation)". So
//! `TaskStatus_t::usStackHighWaterMark` is bytes and needs no scaling.

use core::ffi::CStr;
use std::sync::Mutex;
use std::thread::{Builder, JoinHandle};

use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use esp_idf_svc::sys::{
    CONFIG_ESP_MAIN_TASK_STACK_SIZE, CONFIG_ESP_SYSTEM_EVENT_TASK_STACK_SIZE,
    CONFIG_ESP_TIMER_TASK_STACK_SIZE, CONFIG_FREERTOS_IDLE_TASK_STACKSIZE,
    CONFIG_FREERTOS_MAX_TASK_NAME_LEN, CONFIG_FREERTOS_TIMER_TASK_STACK_DEPTH,
    CONFIG_LWIP_TCPIP_TASK_STACK_SIZE, TaskStatus_t, UBaseType_t, uxTaskGetNumberOfTasks,
    uxTaskGetSystemState,
};

/// The ESP-IDF main task, which this firmware's `main` never leaves: it
/// becomes the heartbeat loop. Its stack is a Kconfig value, not ours.
const MAIN_TASK: &str = "main";

/// Tasks this firmware does not create but whose stack size it can still
/// name: the ESP-IDF ones come straight from `sdkconfig`, and `mqtt_task`
/// is created by esp-mqtt from the size [`crate::mqtt`] hands it.
///
/// Every ESP-IDF stack size here is in bytes: `StackType_t` is `uint8_t`
/// on this port, so a FreeRTOS "stack depth" is a byte count.
///
/// Not in the list: `httpd`. Both HTTP servers (`crate::http`) run a task
/// of that name with different stacks (12288 and 10240), and neither
/// ESP-IDF's `httpd_config_t` nor esp-idf-svc 0.53's `Configuration` has a
/// task-name field, so the report cannot tell the two apart. They are left
/// without a size rather than shown a wrong one.
const IDF_TASKS: &[(&str, usize)] = &[
    // The default event loop, where esp_netif, esp_eth, the IP event
    // watch of `platform::net` and our own handlers all run.
    ("sys_evt", CONFIG_ESP_SYSTEM_EVENT_TASK_STACK_SIZE as usize),
    // lwIP.
    ("tiT", CONFIG_LWIP_TCPIP_TASK_STACK_SIZE as usize),
    // esp_timer callbacks.
    ("esp_timer", CONFIG_ESP_TIMER_TASK_STACK_SIZE as usize),
    // The FreeRTOS software-timer service task.
    ("Tmr Svc", CONFIG_FREERTOS_TIMER_TASK_STACK_DEPTH as usize),
    ("mqtt_task", crate::mqtt::MQTT_TASK_STACK),
];

/// The idle task: `IDLE` on a single-core port, `IDLE0` / `IDLE1` on a
/// dual-core one, so it is matched by prefix.
const IDLE_TASK: &str = "IDLE";

/// Longest task name FreeRTOS keeps, without the NUL.
const NAME_LEN: usize = CONFIG_FREERTOS_MAX_TASK_NAME_LEN as usize - 1;

/// `name -> requested stack size` for the threads this firmware spawns.
/// Small (about 15 entries) and only appended to, by [`spawn`].
static SIZES: Mutex<Vec<(&'static CStr, usize)>> = Mutex::new(Vec::new());

/// Spawn a thread whose FreeRTOS task carries `name` and `stack` bytes of
/// stack, and remember the size so [`report`] can turn a high-water mark
/// into a percentage.
///
/// `name` must fit `configMAX_TASK_NAME_LEN` (15 characters here);
/// anything longer is truncated by FreeRTOS, and the registry lookup then
/// matches on the truncated prefix.
///
/// The esp_pthread configuration is per-thread and sticky, so it is put
/// back to the default before returning, whether or not the spawn worked.
pub fn spawn<F, T>(name: &'static CStr, stack: usize, f: F) -> std::io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let label = name.to_str().unwrap_or("task");
    let conf = ThreadSpawnConfiguration {
        name: Some(name),
        stack_size: stack,
        ..ThreadSpawnConfiguration::default()
    };
    if let Err(e) = conf.set() {
        return Err(std::io::Error::other(format!(
            "esp_pthread_set_cfg for `{label}`: {e}"
        )));
    }
    // `std` sets the stack through pthread_attr_setstacksize, which wins
    // over the esp_pthread value; both are set to the same number so it
    // does not matter which one the port honours.
    let spawned = Builder::new()
        .name(label.to_string())
        .stack_size(stack)
        .spawn(f);
    // Back to the default, so the next plain `thread::spawn` anywhere in
    // this thread is not labelled with our name and sized with our stack.
    let _ = ThreadSpawnConfiguration::default().set();
    if spawned.is_ok() {
        register(name, stack);
    }
    spawned
}

fn register(name: &'static CStr, stack: usize) {
    if let Ok(mut sizes) = SIZES.lock()
        && !sizes.iter().any(|(known, _)| *known == name)
    {
        sizes.push((name, stack));
    }
}

/// One task's stack accounting.
#[derive(Debug, Clone)]
pub struct TaskStack {
    /// The FreeRTOS task name, as the kernel kept it (so truncated to 15
    /// characters if it was longer).
    pub name: String,
    /// High-water mark: the least stack this task has ever had left, in
    /// bytes. The smaller, the closer it came to overflowing.
    pub headroom: u32,
    /// Stack it was given, when we know it: our own threads (recorded by
    /// [`spawn`]), the ESP-IDF main task, and the tasks of `IDF_TASKS`
    /// whose size is a `sdkconfig` constant. `None` for the rest
    /// (`ipc0`, `httpd`, ...).
    pub size: Option<usize>,
}

impl TaskStack {
    /// Percentage of a known stack that has been used at the worst
    /// moment. `None` when the size is not known.
    pub fn used_pct(&self) -> Option<u32> {
        let size = self.size? as u64;
        if size == 0 {
            return None;
        }
        let used = size.saturating_sub(self.headroom as u64);
        Some(((used * 100) / size) as u32)
    }
}

/// Every task in the system with its stack high-water mark in bytes,
/// least headroom first.
///
/// Needs `CONFIG_FREERTOS_USE_TRACE_FACILITY=y`: without it
/// `uxTaskGetSystemState` is declared but not compiled, and the firmware
/// does not link. Suspends the scheduler for its duration, so this is a
/// debug aid - the console `stacks` command and one log line block after
/// boot - not something to poll.
pub fn report() -> Vec<TaskStack> {
    // A few spare slots: a task may be created between the count and the
    // snapshot, and uxTaskGetSystemState returns 0 if the array is short.
    let room = unsafe { uxTaskGetNumberOfTasks() } as usize + 4;
    let mut raw: Vec<TaskStatus_t> = vec![unsafe { core::mem::zeroed() }; room];
    let got = unsafe {
        uxTaskGetSystemState(raw.as_mut_ptr(), room as UBaseType_t, core::ptr::null_mut())
    } as usize;
    let mut out: Vec<TaskStack> = raw[..got.min(room)]
        .iter()
        .map(|task| {
            let name = if task.pcTaskName.is_null() {
                String::from("?")
            } else {
                unsafe { CStr::from_ptr(task.pcTaskName) }
                    .to_string_lossy()
                    .into_owned()
            };
            let size = size_of(&name);
            TaskStack {
                name,
                headroom: task.usStackHighWaterMark,
                size,
            }
        })
        .collect();
    out.sort_by(|a, b| a.headroom.cmp(&b.headroom).then_with(|| a.name.cmp(&b.name)));
    out
}

fn size_of(name: &str) -> Option<usize> {
    if name == MAIN_TASK {
        return Some(CONFIG_ESP_MAIN_TASK_STACK_SIZE as usize);
    }
    if name.starts_with(IDLE_TASK) {
        return Some(CONFIG_FREERTOS_IDLE_TASK_STACKSIZE as usize);
    }
    if let Some((_, size)) = IDF_TASKS.iter().find(|(known, _)| *known == name) {
        return Some(*size);
    }
    let sizes = SIZES.lock().ok()?;
    sizes
        .iter()
        .find(|(known, _)| {
            let known = known.to_bytes();
            &known[..known.len().min(NAME_LEN)] == name.as_bytes()
        })
        .map(|(_, size)| *size)
}

/// [`report`] as printable lines: a header and one line per task, least
/// headroom first. Shared by the console command and the boot-time log
/// block so the two never drift apart.
pub fn lines() -> Vec<String> {
    let tasks = report();
    let mut out = Vec::with_capacity(tasks.len() + 1);
    out.push(format!(
        "{} tasks, least stack headroom first; `free` is the high-water mark in bytes (the least \
         that task ever had left)",
        tasks.len()
    ));
    for task in &tasks {
        match (task.size, task.used_pct()) {
            (Some(size), Some(pct)) => out.push(format!(
                "  {:<15} free {:>6}  of {:>6}  {:>3}% used",
                task.name, task.headroom, size, pct
            )),
            _ => out.push(format!(
                "  {:<15} free {:>6}  of      ?     ?% used",
                task.name, task.headroom
            )),
        }
    }
    out
}
