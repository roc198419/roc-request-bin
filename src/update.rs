use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Ready,
    Checking,
    Current,
    Installed(String),
    Failed(String),
}
impl UpdateStatus {
    pub fn message(&self) -> String {
        match self {
            Self::Ready => "尚未检查更新".into(),
            Self::Checking => "正在检查更新…".into(),
            Self::Current => "已是最新版本".into(),
            Self::Installed(version) => format!("已更新至 {version}，重启后生效"),
            Self::Failed(error) => format!("更新检查失败：{error}"),
        }
    }
    pub fn can_check(&self) -> bool {
        !matches!(self, Self::Checking | Self::Installed(_))
    }
}

// None means current; Some(version) means an update was installed.
type CheckResult = Option<String>;
struct State {
    status: UpdateStatus,
    next_check: Instant,
}
impl State {
    fn begin(&mut self, now: Instant, manual: bool) -> bool {
        if !self.status.can_check() || (!manual && now < self.next_check) {
            return false;
        }
        self.status = UpdateStatus::Checking;
        // Manual checks also postpone the next scheduled check.
        self.next_check = now + CHECK_INTERVAL;
        true
    }
    fn finish(&mut self, result: anyhow::Result<CheckResult>) {
        self.status = match result {
            Ok(None) => UpdateStatus::Current,
            Ok(Some(version)) => UpdateStatus::Installed(version),
            Err(error) => UpdateStatus::Failed(error.to_string()),
        };
    }
}

#[derive(Clone)]
pub struct UpdateManager {
    state: Arc<Mutex<State>>,
    checker: Arc<dyn Fn() -> anyhow::Result<CheckResult> + Send + Sync>,
}
impl UpdateManager {
    #[cfg(feature = "desktop")]
    pub fn new() -> Self {
        Self::with_checker(check_release)
    }

    #[cfg(any(feature = "desktop", test))]
    fn with_checker(
        checker: impl Fn() -> anyhow::Result<CheckResult> + Send + Sync + 'static,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                status: UpdateStatus::Ready,
                next_check: Instant::now(),
            })),
            checker: Arc::new(checker),
        }
    }
    pub fn status(&self) -> UpdateStatus {
        self.state.lock().unwrap().status.clone()
    }
    pub fn check_if_due(&self) -> bool {
        self.start(Instant::now(), false)
    }
    pub fn check_now(&self) -> bool {
        self.start(Instant::now(), true)
    }
    fn start(&self, now: Instant, manual: bool) -> bool {
        if !self.state.lock().unwrap().begin(now, manual) {
            return false;
        }
        let state = self.state.clone();
        let checker = self.checker.clone();
        let spawn = std::thread::Builder::new()
            .name("roc-update-check".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| checker()))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("更新检查异常，请重试")));
                state.lock().unwrap().finish(result);
            });
        if let Err(error) = spawn {
            self.state.lock().unwrap().finish(Err(error.into()));
            return false;
        }
        true
    }
}
#[cfg(feature = "desktop")]
impl Default for UpdateManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "desktop")]
fn check_release() -> anyhow::Result<CheckResult> {
    let update = self_update::backends::github::Update::configure()
        .repo_owner("roc198419")
        .repo_name("roc-request-bin")
        .identifier("roc-request-bin-")
        .bin_name("roc-request-bin")
        .bin_path_in_archive(if cfg!(windows) {
            "roc-request-bin.exe"
        } else {
            "roc-request-bin"
        })
        .current_version(env!("CARGO_PKG_VERSION"))
        .no_confirm(true)
        .show_download_progress(false)
        .build()?
        .update()?;
    Ok(if update.updated() {
        Some(update.version().into())
    } else {
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(now: Instant) -> State {
        State {
            status: UpdateStatus::Ready,
            next_check: now,
        }
    }

    #[test]
    fn startup_and_ten_minute_boundary() {
        let now = Instant::now();
        let mut state = state(now);
        assert!(state.begin(now, false));
        state.finish(Ok(None));
        assert!(!state.begin(now + Duration::from_secs(599), false));
        assert!(state.begin(now + Duration::from_secs(600), false));
    }
    #[test]
    fn manual_check_resets_schedule_and_failures_can_retry() {
        let now = Instant::now();
        let mut state = state(now);
        assert!(state.begin(now, false));
        state.finish(Ok(None));
        assert!(state.begin(now + Duration::from_secs(60), true));
        state.finish(Err(anyhow::anyhow!("offline")));
        assert_eq!(state.status, UpdateStatus::Failed("offline".into()));
        assert!(!state.begin(now + CHECK_INTERVAL, false));
        assert!(state.begin(now + CHECK_INTERVAL + Duration::from_secs(60), false));
        state.finish(Err(anyhow::anyhow!("offline")));
        assert!(state.begin(now + CHECK_INTERVAL + Duration::from_secs(61), true));
    }
    #[test]
    fn installed_update_is_not_installed_again_before_restart() {
        let now = Instant::now();
        let mut state = state(now);
        assert!(state.begin(now, true));
        state.finish(Ok(Some("0.4.0".into())));
        assert!(!state.begin(now + CHECK_INTERVAL * 2, false));
        assert!(!state.begin(now + CHECK_INTERVAL * 2, true));
        assert!(state.status.message().contains("重启后生效"));
    }
    #[test]
    fn manual_and_scheduled_checks_cannot_run_concurrently() {
        use std::sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let started = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let count = Arc::new(AtomicUsize::new(0));
        let manager = UpdateManager::with_checker({
            let (started, release, count) = (started.clone(), release.clone(), count.clone());
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                started.wait();
                release.wait();
                Ok(None)
            }
        });
        assert!(manager.check_now());
        started.wait();
        let manual = manager.check_now();
        let scheduled = manager.start(Instant::now() + CHECK_INTERVAL, false);
        release.wait();
        assert!(!manual && !scheduled);
        let deadline = Instant::now() + Duration::from_secs(2);
        while manager.status() == UpdateStatus::Checking && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(manager.status(), UpdateStatus::Current);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
