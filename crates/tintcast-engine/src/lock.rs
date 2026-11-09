use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::HSTRING;

use crate::EngineError;

#[derive(Debug, Clone)]
pub struct EngineLock {
    name: String,
    timeout: Duration,
}

impl Default for EngineLock {
    fn default() -> Self {
        Self::new(Self::NAME, Self::TIMEOUT)
    }
}

impl EngineLock {
    pub const NAME: &'static str = r"Local\Tintcast.Engine";
    pub const TIMEOUT: Duration = Duration::from_secs(10);

    pub fn new(name: impl Into<String>, timeout: Duration) -> Self {
        Self {
            name: name.into(),
            timeout,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn acquire(&self) -> Result<LockGuard, EngineError> {
        let name = HSTRING::from(self.name.as_str());
        let handle = unsafe { CreateMutexW(None, false, &name) }
            .map_err(|error| EngineError::Lock(error.message()))?;
        let millis = u32::try_from(self.timeout.as_millis()).unwrap_or(u32::MAX);
        let wait = unsafe { WaitForSingleObject(handle, millis) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            return Ok(LockGuard { handle });
        }
        let error = windows::core::Error::from_thread();
        let _ = unsafe { CloseHandle(handle) };
        if wait == WAIT_TIMEOUT {
            Err(EngineError::Busy)
        } else {
            Err(EngineError::Lock(error.message()))
        }
    }
}

#[derive(Debug)]
pub struct LockGuard {
    handle: HANDLE,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    use super::*;

    fn unique_name(label: &str) -> String {
        format!(
            r"Local\Tintcast.Engine.test-{label}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_nanosecond()
        )
    }

    #[test]
    fn the_default_lock_is_the_engine_mutex_with_a_ten_second_wait() {
        let lock = EngineLock::default();
        assert_eq!(lock.name(), r"Local\Tintcast.Engine");
        assert_eq!(lock.timeout(), Duration::from_secs(10));
    }

    #[test]
    fn a_second_holder_waits_then_fails_with_another_apply_is_running() {
        let name = unique_name("busy");
        let (held, wait_for_hold) = mpsc::channel();
        let (release, wait_for_release) = mpsc::channel::<()>();
        let holder_name = name.clone();
        let holder = thread::spawn(move || {
            let guard = EngineLock::new(holder_name, Duration::from_secs(1))
                .acquire()
                .unwrap();
            held.send(()).unwrap();
            wait_for_release.recv().unwrap();
            drop(guard);
        });
        wait_for_hold.recv().unwrap();
        let started = Instant::now();
        let error = EngineLock::new(name.clone(), Duration::from_millis(300))
            .acquire()
            .unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert!(matches!(error, EngineError::Busy));
        assert_eq!(error.to_string(), "Another apply is running.");
        release.send(()).unwrap();
        holder.join().unwrap();
        EngineLock::new(name, Duration::from_millis(300))
            .acquire()
            .unwrap();
    }

    #[test]
    fn a_lock_abandoned_by_a_dead_holder_can_be_taken() {
        let name = unique_name("abandoned");
        let holder_name = name.clone();
        thread::spawn(move || {
            let guard = EngineLock::new(holder_name, Duration::from_secs(1))
                .acquire()
                .unwrap();
            std::mem::forget(guard);
        })
        .join()
        .unwrap();
        EngineLock::new(name, Duration::from_millis(500))
            .acquire()
            .unwrap();
    }
}
