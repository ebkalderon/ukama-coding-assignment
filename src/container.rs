//! Types for creating and controlling running containers.

use std::process::Stdio;

use anyhow::anyhow;
use fallible_collections::tryformat;
use tokio::process::Command;
use tokio_seqpacket::UnixSeqpacket;
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

        println!(
            "spawned conmon for {}, writing start byte...",
            rt.base_dir().display()
        );

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

        eprintln!("wrote start byte, waiting for `conmon` to fork and exec...");

        // Wait for initial setup to complete.
        let output = child.wait_with_output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(anyhow!(
                "failed to create container, `conmon` exited with non-zero status: [{}]",
                stderr
            ));
        }

        eprintln!("reading PID from `conmon`...");
        let pid = sync_pipe.get_pid().await?;
        eprintln!("received PID {}, connecting to console socket...", pid);

        // Setup is complete, so connect to the console socket.
        let sock_path = rt.base_dir().join(uuid_str).join("attach");
        let console_sock = UnixSeqpacket::connect(sock_path).await?;
        eprintln!("connected to console socket!");

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
    pub async fn start(&self) -> anyhow::Result<()> {
        let mut pause_cmd = Command::new(RUNTIME_BIN);
        pause_cmd.args(&["start", &self.id]);
        exec_command(&mut pause_cmd).await?;
        Ok(())
    }

    /// Pause the container's execution, if it currently running.
    pub async fn pause(&self) -> anyhow::Result<()> {
        let mut pause_cmd = Command::new(RUNTIME_BIN);
        pause_cmd.args(&["pause", &self.id]);
        exec_command(&mut pause_cmd).await?;
        Ok(())
    }

    /// Resume the container's execution, if it currently paused.
    pub async fn resume(&self) -> anyhow::Result<()> {
        let mut resume_cmd = Command::new(RUNTIME_BIN);
        resume_cmd.args(&["resume", &self.id]);
        exec_command(&mut resume_cmd).await?;
        Ok(())
    }

    /// Delete the container immediately.
    pub async fn delete(self) -> anyhow::Result<()> {
        let mut delete_cmd = Command::new(RUNTIME_BIN);
        delete_cmd.args(&["delete", "--force", &self.id]);
        exec_command(&mut delete_cmd).await?;
        Ok(())
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

async fn exec_command(cmd: &mut Command) -> anyhow::Result<()> {
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = std::str::from_utf8(&output.stderr)?;
        return Err(anyhow!("`{:?}` returned: [{}]", cmd, stderr));
    }

    Ok(())
}
