// SPDX-License-Identifier: GPL-3.0-or-later
//! The AppKit main thread presents compositor leases; Linux stays thread-affine.
use std::{
    ffi::{c_char, c_void, CString},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use trames_shared::{IoSignal, IoSubscription, IoWake};
use trames_wayland::{ImageBuffer, InputEvent, Server, WindowMode, WindowSnapshot};
#[repr(C)]
struct Image {
    kind: u32,
    address: *mut c_void,
    length: usize,
    offset: u32,
    width: u32,
    height: u32,
    stride: u32,
    alpha: u32,
    x: i32,
    y: i32,
    scale: i32,
    transform: u32,
}
#[repr(C)]
struct Window {
    id: u64,
    revision: u64,
    interaction_serial: u32,
    interaction_edges: u32,
    width: u32,
    height: u32,
    min_width: u32,
    min_height: u32,
    max_width: u32,
    max_height: u32,
    mode: u32,
    title: *const c_char,
}
unsafe extern "C" {
    fn trames_ui_create(shader: *const c_char) -> *mut c_void;
    fn trames_ui_source(ui: *mut c_void) -> *mut c_void;
    fn trames_ui_wake(source: *mut c_void);
    fn trames_ui_release_source(source: *mut c_void);
    fn trames_ui_callbacks(
        ui: *mut c_void,
        context: *const c_void,
        refresh: extern "C" fn(*const c_void),
        event: extern "C" fn(*const c_void, u64, u32, f64, f64, u32, u32, u32),
    );
    fn trames_ui_run(ui: *mut c_void);
    fn trames_ui_finish(ui: *mut c_void);
    fn trames_ui_destroy(ui: *mut c_void);
    fn trames_ui_begin(ui: *mut c_void);
    fn trames_ui_window(
        ui: *mut c_void,
        info: *const Window,
        images: *const Image,
        count: usize,
        lease: *mut c_void,
    ) -> i32;
    fn trames_ui_end(ui: *mut c_void);
}
struct Wake(usize);
impl IoWake for Wake {
    fn wake_io(&self, _: &Arc<IoSignal>) {
        self.notify();
    }
}
impl Wake {
    fn notify(&self) {
        unsafe {
            trames_ui_wake(self.0 as *mut c_void);
        }
    }
}
impl Drop for Wake {
    fn drop(&mut self) {
        unsafe {
            trames_ui_release_source(self.0 as *mut c_void);
        }
    }
}
struct Binding {
    server: Server,
    signal: Arc<IoSignal>,
    _subscription: IoSubscription,
}
pub struct Bridge {
    native: usize,
    wake: Arc<Wake>,
    binding: Mutex<Option<Binding>>,
    failure: Mutex<Option<String>>,
    stopped: AtomicBool,
}
impl Bridge {
    pub fn attach(&self, server: Server) {
        let target: Arc<dyn IoWake> = self.wake.clone();
        let signal = IoSignal::new(0, Arc::downgrade(&target));
        let subscription = server.subscribe(&signal);
        *self.binding.lock().unwrap() = Some(Binding {
            server,
            signal,
            _subscription: subscription,
        });
        self.wake.notify();
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
    fn fail(&self, error: impl ToString) {
        *self.failure.lock().unwrap() = Some(error.to_string());
        self.stopped.store(true, Ordering::Relaxed);
    }
    fn refresh(&self) -> Result<(), String> {
        let binding = self
            .binding
            .lock()
            .unwrap()
            .as_ref()
            .map(|b| (b.server.clone(), b.signal.clone()));
        let Some((server, signal)) = binding else {
            return Ok(());
        };
        signal.reset();
        let snapshot = server.snapshot();
        if std::env::var_os("TRAMES_MAC_WINDOW_TRACE").is_some() {
            eprintln!(
                "trames mac window snapshot: revision={} windows={}",
                snapshot.revision,
                snapshot.windows.len()
            );
        }
        // SAFETY: only the AppKit main callback calls native presentation. No
        // guest/Rust byte reference is constructed; frames own all raw sources.
        unsafe {
            trames_ui_begin(self.native as *mut _);
        }
        for window in snapshot.windows {
            let mut images = Vec::new();
            for image in &window.scene {
                let (width, height) = image.buffer.dimensions();
                let mut info = Image {
                    kind: 0,
                    address: std::ptr::null_mut(),
                    length: 0,
                    offset: 0,
                    width,
                    height,
                    stride: 0,
                    alpha: 0,
                    x: image.x,
                    y: image.y,
                    scale: image.scale,
                    transform: image.transform,
                };
                match &image.buffer {
                    ImageBuffer::Shm(lease) => {
                        let p = &lease.pixels;
                        let map = &p.mapping;
                        info.kind = 1;
                        info.address = map.allocation_pointer().cast();
                        info.length = map.allocation_len();
                        info.offset = map
                            .allocation_offset()
                            .try_into()
                            .map_err(|_| "SHM offset exceeds Metal range")?;
                        info.stride = p.stride;
                        info.alpha = u32::from(p.alpha);
                        let end = map
                            .allocation_offset()
                            .checked_add(p.stride as usize * height as usize)
                            .ok_or("SHM range overflow")?;
                        if end > info.length || end > u32::MAX as usize {
                            return Err("SHM image exceeds retained mapping".into());
                        }
                    }
                    ImageBuffer::Gpu(gpu) => {
                        info.kind = 2;
                        info.address = gpu.metal_texture() as *mut c_void;
                    }
                }
                images.push(info);
            }
            if images.is_empty() {
                continue;
            }
            let title = CString::new(window.title.replace('\0', " ")).map_err(|e| e.to_string())?;
            let info = Window {
                id: window.id,
                revision: window.revision,
                interaction_serial: window.interaction.map_or(0, |i| i.serial),
                interaction_edges: window.interaction.map_or(0, |i| i.edges),
                width: window.width,
                height: window.height,
                min_width: window.min_size.0,
                min_height: window.min_size.1,
                max_width: window.max_size.0,
                max_height: window.max_size.1,
                mode: match window.mode {
                    WindowMode::Floating => 0,
                    WindowMode::Minimized => 1,
                    WindowMode::Maximized => 2,
                },
                title: title.as_ptr(),
            };
            let frame = Box::into_raw(Box::new(Frame {
                server: server.clone(),
                window,
                #[cfg(test)]
                dropped: None,
            }));
            let submitted = unsafe {
                trames_ui_window(
                    self.native as *mut _,
                    &info,
                    images.as_ptr(),
                    images.len(),
                    frame.cast(),
                )
            };
            if submitted <= 0 {
                unsafe {
                    drop(Box::from_raw(frame));
                }
            }
            if submitted < 0 || submitted == 2 {
                return Err("native Metal window presentation failed".into());
            }
        }
        unsafe {
            trames_ui_end(self.native as *mut _);
        }
        Ok(())
    }
}
struct Frame {
    server: Server,
    window: WindowSnapshot,
    #[cfg(test)]
    dropped: Option<Arc<std::sync::atomic::AtomicUsize>>,
}
#[cfg(test)]
impl Drop for Frame {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[no_mangle]
extern "C" fn trames_ui_frame_release(pointer: *mut c_void, presented: u32, milliseconds: u32) {
    // SAFETY: native presentation returns exactly one owned Frame, after all
    // GPU reads complete. A positive acknowledgement also requires scanout.
    let frame = unsafe { Box::from_raw(pointer.cast::<Frame>()) };
    if presented != 0 {
        let _ = frame
            .server
            .presented(frame.window.id, frame.window.revision, milliseconds);
    }
}
extern "C" fn refresh(context: *const c_void) {
    let bridge = unsafe { &*context.cast::<Bridge>() };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bridge.refresh())) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => bridge.fail(error),
        Err(_) => bridge.fail("native window callback panicked"),
    }
}
extern "C" fn event(
    context: *const c_void,
    window: u64,
    kind: u32,
    x: f64,
    y: f64,
    a: u32,
    b: u32,
    c: u32,
) {
    let bridge = unsafe { &*context.cast::<Bridge>() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if kind == 10 {
            bridge.fail("Metal display command failed");
            return Ok(());
        }
        if kind == 9 {
            bridge.stopped.store(true, Ordering::Relaxed);
            return Ok(());
        }
        let server = bridge
            .binding
            .lock()
            .unwrap()
            .as_ref()
            .map(|b| b.server.clone());
        let Some(server) = server else {
            return Ok(());
        };
        let mut clock = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let time = (clock.tv_sec as u64 * 1000 + clock.tv_nsec as u64 / 1_000_000) as u32;
        match kind {
            1 => server.input(InputEvent::Focus {
                window: (window != 0).then_some(window),
            }),
            2 => server.input(InputEvent::Pointer { window, x, y, time }),
            3 => server.input(InputEvent::Button {
                button: a,
                pressed: b != 0,
                time,
            }),
            4 => server.input(InputEvent::Scroll {
                horizontal: x,
                vertical: y,
                time,
            }),
            5 => server.input(InputEvent::Key {
                key: a,
                pressed: b != 0,
                modifiers: c,
                time,
            }),
            6 => server.resize(window, a.clamp(1, 4096), b.clamp(1, 4096)),
            7 => server.close(window),
            8 => server.set_mode(
                window,
                match a {
                    1 => WindowMode::Minimized,
                    2 => WindowMode::Maximized,
                    _ => WindowMode::Floating,
                },
            ),
            _ => Ok(()),
        }
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => bridge.fail(e),
        Err(_) => bridge.fail("native input callback panicked"),
    }
}
pub fn run() -> Result<u8, String> {
    let shader = CString::new(include_str!("window.metal")).unwrap();
    let native = unsafe { trames_ui_create(shader.as_ptr()) };
    if native.is_null() {
        return Err("cannot create main run-loop source".into());
    }
    let source = unsafe { trames_ui_source(native) };
    let bridge = Arc::new(Bridge {
        native: native as usize,
        wake: Arc::new(Wake(source as usize)),
        binding: Mutex::new(None),
        failure: Mutex::new(None),
        stopped: AtomicBool::new(false),
    });
    unsafe {
        trames_ui_callbacks(native, Arc::as_ptr(&bridge).cast(), refresh, event);
    }
    let worker_bridge = bridge.clone();
    let pointer = native as usize;
    let worker = std::thread::Builder::new()
        .name("trames-linux".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::session::run(Some(worker_bridge)).map_err(|e| e.to_string())
            }))
            .unwrap_or_else(|_| Err("Linux session panicked".into()));
            unsafe {
                trames_ui_finish(pointer as *mut c_void);
            }
            result
        })
        .map_err(|e| e.to_string())?;
    unsafe {
        trames_ui_run(native);
    }
    let result = worker
        .join()
        .unwrap_or_else(|_| Err("Linux session panicked".into()));
    unsafe {
        trames_ui_destroy(native);
    }
    let failure = bridge.failure.lock().unwrap().take();
    if let Some(error) = failure {
        Err(error)
    } else if bridge.stopped() {
        Ok(0)
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    unsafe extern "C" {
        fn trames_ui_test_frame(lease: *mut c_void, count: u32) -> *mut c_void;
        fn trames_ui_test_frame_event(frame: *mut c_void, event: u32);
        fn trames_ui_test_frame_state(frame: *mut c_void) -> u32;
        fn trames_ui_test_frame_drop(frame: *mut c_void);
    }
    fn frame() -> (*mut c_void, Arc<AtomicUsize>) {
        let dropped = Arc::new(AtomicUsize::new(0));
        let window = WindowSnapshot {
            id: 7,
            connection: 1,
            title: String::new(),
            app_id: String::new(),
            width: 1,
            height: 1,
            mode: WindowMode::Floating,
            interaction: None,
            parent: None,
            min_size: (0, 0),
            max_size: (0, 0),
            scene: Vec::new(),
            revision: 1,
        };
        let lease = Box::into_raw(Box::new(Frame {
            server: Server::new().unwrap(),
            window,
            dropped: Some(dropped.clone()),
        }));
        (unsafe { trames_ui_test_frame(lease.cast(), 2) }, dropped)
    }
    #[test]
    fn frame_lease_waits_for_gpu_and_every_drawable_and_finishes_once() {
        let (frame, dropped) = frame();
        unsafe {
            trames_ui_test_frame_event(frame, 1);
            trames_ui_test_frame_event(frame, 3);
            assert_eq!(dropped.load(Ordering::Relaxed), 0);
            trames_ui_test_frame_event(frame, 1);
            assert_eq!(trames_ui_test_frame_state(frame), 3);
            assert_eq!(dropped.load(Ordering::Relaxed), 1);
            trames_ui_test_frame_event(frame, 3);
            trames_ui_test_frame_event(frame, 1);
            assert_eq!(dropped.load(Ordering::Relaxed), 1);
            trames_ui_test_frame_drop(frame);
        }
    }
    #[test]
    fn cancelled_and_skipped_frames_release_only_after_gpu_reads_end() {
        for ending in (2..=5).filter(|n| *n != 3) {
            let (frame, dropped) = frame();
            unsafe {
                trames_ui_test_frame_event(frame, ending);
                if ending != 4 {
                    assert_eq!(dropped.load(Ordering::Relaxed), 0);
                    trames_ui_test_frame_event(frame, 3);
                }
                assert_eq!(trames_ui_test_frame_state(frame), 1);
                assert_eq!(dropped.load(Ordering::Relaxed), 1);
                trames_ui_test_frame_drop(frame);
            }
        }
    }
}
