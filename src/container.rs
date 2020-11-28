//! Types for creating and controlling running containers.

use std::path::PathBuf;
use std::process::Stdio;

use anyhow::anyhow;
use fallible_collections::tryformat;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio_seqpacket::UnixSeqpacket;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::image::OciBundle;
use crate::pipe::{CommandExt, StartPipe, SyncPipe};

const CONMON_BIN: &str = "conmon";
const RUNTIME_BIN: &str = "/usr/bin/crun";

/// An actively running OCI container.
#[derive(Debug)]
pub struct Container {
    id: String,
    uuid: Uuid,
    pid: i32,
    console_sock: UnixSeqpacket,
    sync_pipe: SyncPipe,
    runtime: OciBundle,
}

impl Container {
    /// Spawns a new container with the given `id` from the `rt` OCI bundle.
    #[instrument(level = "debug", skip(rt), err)]
    pub async fn create(id: &str, rt: OciBundle) -> anyhow::Result<Self> {
        let id = tryformat!(64, "{}", id).map_err(|e| anyhow!("OOM error: {:?}", e))?;
        let uuid = Uuid::new_v4();
        let uuid_str = tryformat!(36, "{}", uuid).map_err(|e| anyhow!("OOM error: {:?}", e))?;

        let bundle_dir = rt.bundle_dir.to_str().expect("$TMPDIR is invalid UTF-8");
        let exits_dir = rt.exits_dir.to_str().expect("$TMPDIR is invalid UTF-8");
        let log_file = rt.log_file.to_str().expect("$TMPDIR is invalid UTF-8");
        let pid_file = rt.pid_file.to_str().expect("$TMPDIR is invalid UTF-8");
        let sock_dir = rt.base_dir().to_str().expect("$TMPDIR is invalid UTF-8");

        let start_pipe = StartPipe::new()?;
        let mut sync_pipe = SyncPipe::new()?;

        // Spin up the `conmon` child process.
        let child = Command::new(CONMON_BIN)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("--log-level=debug")
            .arg("--systemd-cgroup") // Required for rootless pause/resume.
            .arg("--terminal") // Passes `--console-sock` to `crun`.
            .args(&["--cid", &id])
            .args(&["--cuuid", &uuid_str])
            .args(&["--name", &id])
            .args(&["--runtime", RUNTIME_BIN])
            .args(&["--bundle", bundle_dir])
            .args(&["--exit-dir", exits_dir])
            .args(&["--log-path", log_file])
            .args(&["--container-pidfile", pid_file])
            .args(&["--socket-dir-path", sock_dir])
            .inherit_oci_pipes(&start_pipe, &sync_pipe)
            .spawn()?;

        debug!("spawned `conmon`, signaling ready for setup");
        if let Err(e) = start_pipe.ready().await {
            let output = child.wait_with_output().await?;
            if output.status.success() {
                return Err(e);
            } else {
                let stderr = String::from_utf8(output.stderr)?;
                return Err(anyhow!(
                    "{}, `conmon` exited with non-zero status: [{}]",
                    e,
                    stderr
                ));
            }
        }

        debug!("waiting for `conmon` to complete initial setup");
        let output = child.wait_with_output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(anyhow!(
                "failed to create container, `conmon` exited with non-zero status: [{}]",
                stderr
            ));
        }

        let pid = sync_pipe.get_pid().await?;
        debug!("received container PID from `conmon`: {}", pid);

        // Setup is complete, so connect to the console socket.
        let sock_path = rt.base_dir().join(uuid_str).join("attach");
        debug!("connecting to console socket: {}", sock_path.display());
        let console_sock = UnixSeqpacket::connect(&sock_path).await?;
        debug!("connected to console socket: {}", sock_path.display());
        info!("container has been created with PID {}", pid);

        Ok(Container {
            id,
            uuid,
            pid,
            console_sock,
            sync_pipe,
            runtime: rt,
        })
    }

    /// Start the container, if it isn't already running.
    #[instrument(level = "info", skip(self), fields(id = self.id.as_str(), pid = self.pid, err))]
    pub async fn start(&self) -> anyhow::Result<()> {
        info!("starting container");
        let mut pause_cmd = Command::new(RUNTIME_BIN);
        pause_cmd.args(&["start", &self.id]);
        exec_command(&mut pause_cmd).await?;
        Ok(())
    }

    /// Pause the container's execution, if it currently running.
    #[instrument(level = "info", skip(self), fields(id = self.id.as_str(), pid = self.pid, err))]
    pub async fn pause(&self) -> anyhow::Result<()> {
        info!("pausing container");
        let mut pause_cmd = Command::new(RUNTIME_BIN);
        pause_cmd.args(&["pause", &self.id]);
        exec_command(&mut pause_cmd).await?;
        Ok(())
    }

    /// Resume the container's execution, if it currently paused.
    #[instrument(level = "info", skip(self), fields(id = self.id.as_str(), pid = self.pid, err))]
    pub async fn resume(&self) -> anyhow::Result<()> {
        info!("resuming container");
        let mut resume_cmd = Command::new(RUNTIME_BIN);
        resume_cmd.args(&["resume", &self.id]);
        exec_command(&mut resume_cmd).await?;
        Ok(())
    }

    /// Delete the container immediately.
    #[instrument(level = "info", skip(self), fields(id = self.id.as_str(), pid = self.pid, err))]
    pub async fn delete(self) -> anyhow::Result<()> {
        info!("deleting container");
        let mut delete_cmd = Command::new(RUNTIME_BIN);
        delete_cmd.args(&["delete", "--force", &self.id]);
        exec_command(&mut delete_cmd).await?;
        Ok(())
    }

    /// Retrieves the current state of the container.
    #[instrument(level = "info", skip(self), fields(id = self.id.as_str(), pid = self.pid, err))]
    pub async fn state(&self) -> anyhow::Result<State> {
        info!("retrieving container state");
        let mut state_cmd = Command::new(RUNTIME_BIN);
        state_cmd.args(&["state", &self.id]);

        let state = match exec_command(&mut state_cmd).await {
            Ok(stdout) => serde_json::from_slice(&stdout)?,
            Err(_) => self.read_state_from_exit_file().await?,
        };

        Ok(state)
    }

    /// Retrieves the final state from the exit file, assuming that the container is stopped.
    async fn read_state_from_exit_file(&self) -> anyhow::Result<State> {
        let exit_file = self.runtime.exits_dir.join("exit");
        if !exit_file.exists() {
            return Err(anyhow!(
                "exit file doesn't exist for {} at {}",
                self.id,
                exit_file.display()
            ));
        }

        let bytes = tokio::fs::read(&exit_file).await?;
        let string = String::from_utf8(bytes)?;
        let exit_code = string.parse()?;

        Ok(State {
            id: self.id.clone(),
            status: Status::Stopped { exit_code },
            bundle: self.runtime.bundle_dir.clone(),
        })
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        std::process::Command::new(RUNTIME_BIN)
            .args(&["delete", "--force", &self.id])
            .status()
            .ok();
    }
}

async fn exec_command(cmd: &mut Command) -> anyhow::Result<Vec<u8>> {
    debug!("executing runtime command: {:?}", cmd);

    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = std::str::from_utf8(&output.stderr)?;
        return Err(anyhow!("`{:?}` returned: [{}]", cmd, stderr));
    }

    Ok(output.stdout)
}

/// A list of possible states that the container can be in.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Status {
    Creating,
    Created { pid: u64 },
    Running { pid: u64 },
    Paused { pid: u64 },
    Stopped { exit_code: i64 },
}

/// Represents the current state of a container.
///
/// Based on `state-schema.json` from [opencontainers/runtime-spec].
///
/// [opencontainers/runtime-spec]: https://github.com/opencontainers/runtime-spec/blob/master/schema/state-schema.json
#[derive(Debug, Deserialize, Serialize)]
pub struct State {
    /// The container ID.
    pub id: String,
    /// The current status of the container.
    #[serde(flatten)]
    pub status: Status,
    /// The path to the OCI bundle directory.
    pub bundle: PathBuf,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_creating_state() {
        let _state: State = serde_json::from_value(json!({
            "id": "busybox",
            "status": "creating",
            "bundle":"/tmp/.tmpL0EsKy/bundle"
        }))
        .unwrap();
    }

    #[test]
    fn parses_created_state() {
        let _state: State = serde_json::from_value(json!({
            "id": "busybox",
            "status": "created",
            "pid": 168495,
            "bundle":"/tmp/.tmpL0EsKy/bundle"
        }))
        .unwrap();
    }

    #[test]
    fn parses_running_state() {
        let _state: State = serde_json::from_value(json!({
            "id": "busybox",
            "status": "running",
            "pid": 168495,
            "bundle":"/tmp/.tmpL0EsKy/bundle"
        }))
        .unwrap();
    }

    #[test]
    fn parses_paused_state() {
        let _state: State = serde_json::from_value(json!({
            "id": "busybox",
            "status": "paused",
            "pid": 168495,
            "bundle":"/tmp/.tmpL0EsKy/bundle"
        }))
        .unwrap();
    }

    #[test]
    fn parses_stopped_state() {
        let _state: State = serde_json::from_value(json!({
            "id": "busybox",
            "status": "stopped",
            "exit_code": 0,
            "pid": 168495,
            "bundle":"/tmp/.tmpL0EsKy/bundle"
        }))
        .unwrap();
    }
}
