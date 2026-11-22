//! Manages the local `llama-server` subprocess lifecycle.
//!
//! Callers borrow the engine through [`EngineSupervisor::acquire`], which lazily
//! starts or reuses the process and returns an [`EngineGuard`] lease. The process
//! stops automatically after an idle period or explicitly via [`EngineSupervisor::stop`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;

use crate::ai::binary;
use crate::error::{Error, Result};

mod command;
mod guard;
mod idle;
mod logs;
mod server;

pub use command::{EngineStartOverrides, DEFAULT_CTX_SIZE};
pub use guard::EngineGuard;

use logs::EngineLogs;
use server::LlamaServer;

// Large local models often take 30–60s to load on CPU.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(90);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Lifecycle state of the local engine process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    #[default]
    Stopped,
    Starting,
    Running,
    Error,
}

/// Runtime snapshot exposed to the frontend.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuntimeStatus {
    pub state: EngineState,
    /// Listening port while `Starting` or `Running`.
    pub port: Option<u16>,
    /// Failure summary (including log tail) while in `Error`.
    pub error: Option<String>,
    /// Seconds until automatic idle shutdown when `Running` with no active leases.
    pub idle_seconds_remaining: Option<u64>,
}

/// Singleton supervisor for the local `llama-server` process.
pub struct EngineSupervisor {
    // Lock order: `guards.acquire_lock` -> `state` -> `guards.holders` (never reverse).
    // `guards.holders` uses `std::sync::Mutex` so `EngineGuard::drop` can update counts synchronously.
    state: Mutex<SupervisorState>,
    logs: EngineLogs,
    guards: guard::Guards,
}

#[derive(Default)]
struct SupervisorState {
    slot: EngineSlot,
    // Incremented on every spawn and stop so a superseded `start()` cannot clobber the new slot.
    generation: u64,
}

#[derive(Default)]
enum EngineSlot {
    #[default]
    Stopped,
    Starting(LlamaServer),
    Running(LlamaServer),
    Failed(String),
}

struct HolderState {
    active_count: usize,
    last_used_at: Instant,
}

impl Default for HolderState {
    fn default() -> Self {
        Self {
            active_count: 0,
            last_used_at: Instant::now(),
        }
    }
}

impl Default for EngineSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineSupervisor {
    /// Creates a stopped supervisor.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SupervisorState::default()),
            logs: EngineLogs::new(),
            guards: guard::Guards::default(),
        }
    }

    /// Returns a snapshot of the engine state, including the idle countdown while running.
    pub async fn status(&self) -> EngineRuntimeStatus {
        let state = self.state.lock().await;
        match &state.slot {
            EngineSlot::Stopped => EngineRuntimeStatus::default(),
            EngineSlot::Starting(server) => EngineRuntimeStatus {
                state: EngineState::Starting,
                port: Some(server.loaded.port),
                ..Default::default()
            },
            EngineSlot::Running(server) => EngineRuntimeStatus {
                state: EngineState::Running,
                port: Some(server.loaded.port),
                idle_seconds_remaining: self.idle_eta().map(|remaining| remaining.as_secs()),
                ..Default::default()
            },
            EngineSlot::Failed(message) => EngineRuntimeStatus {
                state: EngineState::Error,
                error: Some(message.clone()),
                ..Default::default()
            },
        }
    }

    /// Stops any starting or running subprocess and resets the slot to `Stopped`.
    ///
    /// Releases `state` before killing so a slow exit does not block `status()` or app shutdown.
    pub async fn stop(&self) -> Result<()> {
        let slot = {
            let mut state = self.state.lock().await;
            // Advance `generation` so any in-flight `start()` exits without overwriting `Stopped`.
            state.generation = state.generation.wrapping_add(1);
            std::mem::take(&mut state.slot)
        };
        if let EngineSlot::Starting(server) | EngineSlot::Running(server) = slot {
            server.kill().await;
        }
        Ok(())
    }

    async fn start(&self, model: PathBuf, overrides: EngineStartOverrides) -> Result<u16> {
        // Hold `state` only to spawn and enter `Starting`; poll `/health` outside the lock.
        let (port, launch_gen) = {
            let mut state = self.state.lock().await;
            if matches!(state.slot, EngineSlot::Starting(_) | EngineSlot::Running(_)) {
                return Err(Error::EngineBusy);
            }
            let binary_path = binary::binary_path()?;
            if !binary_path.exists() {
                let message = "[AI_ENGINE_MISSING] llama-server is not installed".to_string();
                state.slot = EngineSlot::Failed(message.clone());
                return Err(Error::EngineStart(message));
            }
            let server = match LlamaServer::spawn(&binary_path, model, overrides, &self.logs).await
            {
                Ok(server) => server,
                Err(err) => {
                    state.slot = EngineSlot::Failed(format!("Could not start llama-server: {err}"));
                    return Err(Error::Io(err));
                }
            };
            let port = server.loaded.port;
            state.slot = EngineSlot::Starting(server);
            state.generation = state.generation.wrapping_add(1);
            (port, state.generation)
        };

        let healthy = self.wait_until_healthy(port, launch_gen).await;

        let mut state = self.state.lock().await;
        let server = match std::mem::take(&mut state.slot) {
            EngineSlot::Starting(server) if state.generation == launch_gen => server,
            other => {
                // `stop()` or a newer `start()` took over while waiting; leave the slot untouched.
                state.slot = other;
                return Err(Error::EngineStart(
                    "The engine was stopped or restarted while it was starting".to_string(),
                ));
            }
        };
        if healthy {
            state.slot = EngineSlot::Running(server);
            // Start idle countdown from readiness; the lease count is already held by `acquire()`.
            if let Ok(mut holders) = self.guards.holders.lock() {
                holders.last_used_at = Instant::now();
            }
            Ok(port)
        } else {
            let message = server.failure_reason(&self.logs).await;
            state.slot = EngineSlot::Failed(message.clone());
            Err(Error::EngineStart(message))
        }
    }

    // Polls `/health` until 2xx or `HEALTH_TIMEOUT`, exiting early if the process crashes or is
    // superseded so callers do not wait out the full 90s timeout.
    async fn wait_until_healthy(&self, port: u16, launch_gen: u64) -> bool {
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        let health_url = format!("http://127.0.0.1:{port}/health");
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            Ok(client) => client,
            Err(_) => return false,
        };
        while Instant::now() < deadline {
            if !self.is_still_starting(launch_gen).await {
                return false;
            }
            if let Ok(resp) = client.get(&health_url).send().await {
                if resp.status().is_success() {
                    return true;
                }
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
        false
    }

    async fn is_still_starting(&self, launch_gen: u64) -> bool {
        let mut state = self.state.lock().await;
        if state.generation != launch_gen {
            return false;
        }
        match &mut state.slot {
            EngineSlot::Starting(server) => !server.exited(),
            _ => false,
        }
    }
}
