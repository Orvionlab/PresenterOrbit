//! Decisions of the Windows output adapter that need no operating-system call.
use orbit_host::{DisplayBounds, PhysicalSize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// `CW_USEDEFAULT`: let Windows choose the position of a windowed output.
pub const USE_DEFAULT: i32 = i32::MIN;
pub const CONTROL_SIZE: u32 = 38;
const DEFAULT_SIZE: (u32, u32) = (960, 540);

pub struct WindowSpec {
    pub bounds: DisplayBounds,
    pub borderless: bool,
}

/// A display output gives its bounds and is borderless; its close control is a fixed square at the origin until
/// the host places it; a plain window takes the default position and size.
pub fn window_spec(bounds: Option<DisplayBounds>, is_control: bool) -> WindowSpec {
    WindowSpec {
        borderless: is_control || bounds.is_some(),
        bounds: if is_control {
            DisplayBounds {
                x: 0,
                y: 0,
                width: CONTROL_SIZE,
                height: CONTROL_SIZE,
            }
        } else {
            bounds.unwrap_or(DisplayBounds {
                x: USE_DEFAULT,
                y: USE_DEFAULT,
                width: DEFAULT_SIZE.0,
                height: DEFAULT_SIZE.1,
            })
        },
    }
}

/// The size of a native rectangle; a minimized or collapsed window reports at least one pixel.
pub fn physical_size(left: i32, top: i32, right: i32, bottom: i32) -> PhysicalSize<u32> {
    PhysicalSize::new((right - left).max(1) as u32, (bottom - top).max(1) as u32)
}

/// Coalesces display-topology notifications into one refresh pass at a time. Every top-level window receives
/// each notification, and a pass takes host GPU locks, so a burst must neither queue passes nor lose the last one.
#[derive(Default)]
pub struct Coalescer {
    running: AtomicBool,
    generation: AtomicU64,
}

impl Coalescer {
    pub const fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }
    /// Records that the topology changed.
    pub fn changed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    /// True when the caller now owns the pass and must run it.
    pub fn start(&self) -> bool {
        !self.running.swap(true, Ordering::AcqRel)
    }
    /// The generation a pass is about to refresh for.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    /// Ends a pass that refreshed for `generation`. True when the topology changed meanwhile and the caller
    /// still owns the next pass.
    pub fn finish(&self, generation: u64) -> bool {
        self.running.store(false, Ordering::Release);
        self.generation.load(Ordering::Acquire) != generation
            && !self.running.swap(true, Ordering::AcqRel)
    }
    /// Gives up ownership when the pass could not be spawned.
    pub fn abandon(&self) {
        self.running.store(false, Ordering::Release);
    }
}
