//! Background jobs with progress and cancellation (export, import, time
//! stretch, waveform generation). CPU-heavy jobs share a small bounded pool
//! so an 8 GB / few-core machine stays responsive.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum JobState {
    Queued,
    Running,
    Done { result: Value },
    Failed { error: String },
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub progress: f32,
    #[serde(flatten)]
    pub state: JobState,
}

pub struct JobHandle {
    progress: AtomicU32,
    cancel: AtomicBool,
    state: Mutex<JobState>,
    pub id: u64,
    pub kind: String,
    pub label: String,
}

impl JobHandle {
    pub fn set_progress(&self, p: f32) {
        self.progress.store(p.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// Progress callback: returns false when cancelled.
    pub fn report(&self, p: f32) -> bool {
        self.set_progress(p);
        !self.cancelled()
    }
    fn info(&self) -> JobInfo {
        JobInfo {
            id: self.id,
            kind: self.kind.clone(),
            label: self.label.clone(),
            progress: f32::from_bits(self.progress.load(Ordering::Relaxed)),
            state: self.state.lock().clone(),
        }
    }
}

type Task = Box<dyn FnOnce(&JobHandle) -> Result<Value, String> + Send>;

struct Queue {
    items: VecDeque<(Arc<JobHandle>, Task)>,
    shutdown: bool,
}

pub struct Jobs {
    next: AtomicU64,
    jobs: Mutex<Vec<Arc<JobHandle>>>,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl Jobs {
    /// `workers` = number of concurrent CPU jobs.
    pub fn new(workers: usize) -> Self {
        let queue = Arc::new((Mutex::new(Queue { items: VecDeque::new(), shutdown: false }), Condvar::new()));
        let mut handles = Vec::new();
        for i in 0..workers.max(1) {
            let q = queue.clone();
            handles.push(
                std::thread::Builder::new()
                    .name(format!("uta-job-{i}"))
                    .spawn(move || worker(q))
                    .expect("spawn job worker"),
            );
        }
        Self {
            next: AtomicU64::new(1),
            jobs: Mutex::new(Vec::new()),
            queue,
            workers: Mutex::new(handles),
        }
    }

    pub fn submit(&self, kind: &str, label: impl Into<String>, task: impl FnOnce(&JobHandle) -> Result<Value, String> + Send + 'static) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let h = Arc::new(JobHandle {
            progress: AtomicU32::new(0),
            cancel: AtomicBool::new(false),
            state: Mutex::new(JobState::Queued),
            id,
            kind: kind.into(),
            label: label.into(),
        });
        {
            let mut jobs = self.jobs.lock();
            jobs.push(h.clone());
            // keep the list bounded: drop old finished jobs
            if jobs.len() > 100 {
                jobs.retain(|j| matches!(*j.state.lock(), JobState::Queued | JobState::Running));
            }
        }
        let (lock, cv) = &*self.queue;
        lock.lock().items.push_back((h, Box::new(task)));
        cv.notify_one();
        id
    }

    pub fn list(&self) -> Vec<JobInfo> {
        self.jobs.lock().iter().map(|j| j.info()).collect()
    }

    pub fn get(&self, id: u64) -> Option<JobInfo> {
        self.jobs.lock().iter().find(|j| j.id == id).map(|j| j.info())
    }

    pub fn cancel(&self, id: u64) -> bool {
        if let Some(j) = self.jobs.lock().iter().find(|j| j.id == id) {
            j.cancel.store(true, Ordering::Relaxed);
            let mut st = j.state.lock();
            if *st == JobState::Queued {
                *st = JobState::Cancelled;
            }
            return true;
        }
        false
    }

    /// Remove finished jobs from the list.
    pub fn clear_finished(&self) {
        self.jobs
            .lock()
            .retain(|j| matches!(*j.state.lock(), JobState::Queued | JobState::Running));
    }

    /// Block until a job finishes (tests).
    pub fn wait(&self, id: u64, timeout: std::time::Duration) -> Option<JobInfo> {
        let end = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < end {
            if let Some(i) = self.get(id) {
                if !matches!(i.state, JobState::Queued | JobState::Running) {
                    return Some(i);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.get(id)
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        {
            let (lock, cv) = &*self.queue;
            let mut q = lock.lock();
            q.shutdown = true;
            for (h, _) in q.items.drain(..) {
                *h.state.lock() = JobState::Cancelled;
            }
            cv.notify_all();
        }
        for j in self.jobs.lock().iter() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        for h in self.workers.lock().drain(..) {
            let _ = h.join();
        }
    }
}

fn worker(q: Arc<(Mutex<Queue>, Condvar)>) {
    let (lock, cv) = &*q;
    loop {
        let (h, task) = {
            let mut guard = lock.lock();
            loop {
                if guard.shutdown {
                    return;
                }
                if let Some(item) = guard.items.pop_front() {
                    break item;
                }
                cv.wait(&mut guard);
            }
        };
        if h.cancelled() {
            *h.state.lock() = JobState::Cancelled;
            continue;
        }
        *h.state.lock() = JobState::Running;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(&h)));
        let state = match result {
            Ok(Ok(v)) => {
                h.set_progress(1.0);
                JobState::Done { result: v }
            }
            Ok(Err(e)) if h.cancelled() => {
                tracing::info!("job {} cancelled: {e}", h.id);
                JobState::Cancelled
            }
            Ok(Err(e)) => JobState::Failed { error: e },
            Err(_) => JobState::Failed { error: "internal error (the job panicked)".into() },
        };
        *h.state.lock() = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn runs_reports_and_cancels() {
        let jobs = Jobs::new(1);
        let ok = jobs.submit("t", "ok", |h| {
            h.set_progress(0.5);
            Ok(serde_json::json!(42))
        });
        let fail = jobs.submit("t", "fail", |_| Err("nope".into()));
        let panics = jobs.submit("t", "panic", |_| panic!("boom"));
        let slow = jobs.submit("t", "slow", |h| {
            for i in 0..1000 {
                if !h.report(i as f32 / 1000.0) {
                    return Err("cancelled".into());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(Value::Null)
        });
        assert!(matches!(jobs.wait(ok, Duration::from_secs(5)).unwrap().state, JobState::Done { .. }));
        assert!(matches!(jobs.wait(fail, Duration::from_secs(5)).unwrap().state, JobState::Failed { .. }));
        assert!(matches!(jobs.wait(panics, Duration::from_secs(5)).unwrap().state, JobState::Failed { .. }));
        std::thread::sleep(Duration::from_millis(20));
        jobs.cancel(slow);
        assert_eq!(jobs.wait(slow, Duration::from_secs(5)).unwrap().state, JobState::Cancelled);
    }
}
