//! Types for creating and controlling running containers.

use std::process::Stdio;

use anyhow::anyhow;
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
    name: String,
    uuid: Uuid,
    pid: i32,
    console_sock: UnixSeqpacket,
    sync_pipe: SyncPipe,
    runtime: OciBundle,
}

impl Container {
    /// Spawns a new container with the given `id` from the `rt` OCI bundle.
    pub async fn create(id: &str, rt: OciBundle) -> anyhow::Result<Self> {
        let uuid = Uuid::new_v4();

        let start_pipe = StartPipe::new()?;
        let mut sync_pipe = SyncPipe::new()?;

        // Spin up the `conmon` child process.
        let child = Command::new(CONMON_BIN)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(&["--syslog", "--log-level=debug"])
            .arg("--terminal") // Passes `--console-sock` to `crun`.
            .args(&["--cid", id])
            .args(&["--cuuid", &uuid.to_string()])
            .args(&["--name", id])
            .args(&["--runtime", RUNTIME_BIN])
            .args(&["--runtime-arg", "--rootless=true"])
            .args(&["--bundle", &rt.bundle_dir.display().to_string()])
            .args(&["--exit-dir", &rt.exits_dir.display().to_string()])
            .args(&["--log-path", &rt.log_file.display().to_string()])
            .args(&["--container-pidfile", &rt.pid_file.display().to_string()])
            .args(&["--socket-dir-path", &rt.base_dir().display().to_string()])
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
        let sock_path = rt.base_dir().join(uuid.to_string()).join("attach");
        let console_sock = UnixSeqpacket::connect(sock_path).await?;
        eprintln!("connected to console socket!");

        Ok(Container {
            name: id.to_string(),
            uuid,
            pid,
            console_sock,
            sync_pipe,
            runtime: rt,
        })
    }

    /// Returns the name of the container.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the UUIDv4 assigned to the container.
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
    }
}
