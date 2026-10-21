//! Producer side of the AsciiCam shared-memory interface.
//!
//! The consumer (camera DLL, `FilterSubscriber`) creates and initializes every object; the
//! producer only opens them. The camera DLL runs in the Frame Server service (session 0), so the
//! names need the `Global\` prefix to be visible from the user session:
//!
//! | Object  | Name                | Meaning                                              |
//! |---------|---------------------|------------------------------------------------------|
//! | Mapping | `Global\AsciiCam`   | [`SharedFilterState`]                                |
//! | Event   | `Global\AsciiCamPublished` | auto-reset; producer -> consumer, "new value written" |
//! | Event   | `Global\AsciiCamConsumed` | auto-reset; consumer -> producer, "value applied"     |
//!
//! `AsciiCamConsumed` is created signaled, so the first publish does not block. Every publish is
//! `wait(Consumed)` -> write value -> bump sequence -> fence -> `set(Published)`.

use std::fmt;
use std::sync::atomic::{fence, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
    MEMORY_MAPPED_VIEW_ADDRESS,
};
use windows::Win32::System::Threading::{
    OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE,
};

const MAP_NAME: PCWSTR = w!("Global\\AsciiCam");
const PUBLISHED_NAME: PCWSTR = w!("Global\\AsciiCamPublished");
const CONSUMED_NAME: PCWSTR = w!("Global\\AsciiCamConsumed");

pub const MAGIC: u32 = 0x4153_4349;
pub const VERSION: u32 = 1;

/// Highest valid FilterOption value (`0 = Passthrough, 1 = ASCII, 2 = Edge, 3 = Grayscale`).
pub const MAX_FILTER_VALUE: u32 = 3;

/// Must match the C++ `SharedFilterState` field for field (4 x u32 = 16 bytes).
#[repr(C)]
struct SharedFilterState {
    magic: AtomicU32,
    version: AtomicU32,
    value: AtomicU32,
    sequence: AtomicU32,
}

#[derive(Debug)]
pub enum PublishError {
    /// The consumer has not created/initialized the shared objects yet. Retry later.
    ConsumerUnavailable(String),
    /// The consumer did not signal `AsciiCamConsumed` in time; nothing was written. Retry later.
    Timeout,
    /// Protocol mismatch, invalid value, or an API failure. Retrying will not help.
    Failed(String),
}

impl PublishError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::ConsumerUnavailable(_) | Self::Timeout)
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConsumerUnavailable(why) => write!(f, "waiting for AsciiCam ({why})"),
            Self::Timeout => write!(f, "AsciiCam is not responding"),
            Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// Handles opened from the consumer's objects. `Drop` closes whatever was opened, so a partial
/// failure while attaching cleans up automatically.
struct Attached {
    map: HANDLE,
    view: *mut SharedFilterState,
    published: HANDLE,
    consumed: HANDLE,
}

impl Attached {
    fn open() -> Result<Self, PublishError> {
        use PublishError::{ConsumerUnavailable, Failed};

        let mut a = Attached {
            map: HANDLE::default(),
            view: std::ptr::null_mut(),
            published: HANDLE::default(),
            consumed: HANDLE::default(),
        };

        unsafe {
            a.map = OpenFileMappingW(FILE_MAP_READ.0 | FILE_MAP_WRITE.0, false, MAP_NAME)
                .map_err(|e| ConsumerUnavailable(format!("OpenFileMapping: {e}")))?;

            let view = MapViewOfFile(
                a.map,
                FILE_MAP_WRITE,
                0,
                0,
                std::mem::size_of::<SharedFilterState>(),
            );
            if view.Value.is_null() {
                return Err(Failed(format!(
                    "MapViewOfFile: {}",
                    windows::core::Error::from_win32()
                )));
            }
            a.view = view.Value as *mut SharedFilterState;

            let state = &*a.view;
            let magic = state.magic.load(Ordering::Acquire);
            if magic == 0 {
                return Err(ConsumerUnavailable("shared memory not initialized".into()));
            }
            if magic != MAGIC {
                return Err(Failed(format!("bad shared-memory magic 0x{magic:08X}")));
            }
            let version = state.version.load(Ordering::Acquire);
            if version != VERSION {
                return Err(Failed(format!(
                    "protocol version {version}, expected {VERSION}"
                )));
            }

            a.published = OpenEventW(EVENT_MODIFY_STATE, false, PUBLISHED_NAME)
                .map_err(|e| ConsumerUnavailable(format!("OpenEvent Published: {e}")))?;
            a.consumed = OpenEventW(
                SYNCHRONIZATION_SYNCHRONIZE | EVENT_MODIFY_STATE,
                false,
                CONSUMED_NAME,
            )
                .map_err(|e| ConsumerUnavailable(format!("OpenEvent Consumed: {e}")))?;
        }
        Ok(a)
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        // Closing our handles never destroys objects the consumer still has open.
        unsafe {
            if !self.view.is_null() {
                let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.view as _,
                });
            }
            for handle in [self.map, self.published, self.consumed] {
                if !handle.is_invalid() {
                    let _ = CloseHandle(handle);
                }
            }
        }
    }
}

pub struct FilterPublisher {
    attached: Option<Attached>,
}

impl FilterPublisher {
    pub fn new() -> Self {
        Self { attached: None }
    }

    /// Publishes `value` (a FilterOption, `0..=3`). Attaches to the consumer on first use and
    /// keeps retrying the attach on later calls. If the previous value has not been consumed
    /// within `timeout_ms`, nothing is written and `Timeout` is returned.
    pub fn publish(&mut self, value: u32, timeout_ms: u32) -> Result<(), PublishError> {
        if value > MAX_FILTER_VALUE {
            return Err(PublishError::Failed(format!("invalid FilterOption {value}")));
        }
        if self.attached.is_none() {
            self.attached = Some(Attached::open()?);
        }
        let a = self.attached.as_ref().expect("attached above");

        unsafe {
            // 1. The previous value must have been applied before it may be overwritten.
            let wait = WaitForSingleObject(a.consumed, timeout_ms);
            if wait == WAIT_TIMEOUT {
                return Err(PublishError::Timeout);
            }
            if wait != WAIT_OBJECT_0 {
                return Err(PublishError::Failed(format!("wait on Consumed returned {}", wait.0)));
            }

            // 2-3. Write the value and bump the publish counter.
            let state = &*a.view;
            state.value.store(value, Ordering::Release);
            state.sequence.fetch_add(1, Ordering::Release);

            // 4. Make the writes visible before the consumer is woken.
            fence(Ordering::SeqCst);

            // 5. Notify the consumer.
            if let Err(e) = SetEvent(a.published) {
                // The Consumed token was taken but nothing was published; give it back so the
                // next publish does not wait for an acknowledgement that will never come.
                let _ = SetEvent(a.consumed);
                return Err(PublishError::Failed(format!("SetEvent Published: {e}")));
            }
        }
        Ok(())
    }
}
