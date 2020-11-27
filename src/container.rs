use std::process::Stdio;

use anyhow::anyhow;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_seqpacket::UnixSeqpacket;
use uuid::Uuid;

use crate::pipe::{CommandExt, PipeReader, PipeWriter};
use crate::RuntimeDir;

#[derive(Debug)]
pub struct Container {
    name: String,
    uuid: Uuid,
    pid: i32,
    console_sock: UnixSeqpacket,
    sync_pipe: BufReader<PipeReader>,
    runtime_dir: RuntimeDir,
}

impl Container {
    pub async fn create(id: &str, root: RuntimeDir) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum SyncInfo {
            Err { pid: i32, message: String },
            Ok { pid: i32 },
        }

        let container_uuid = Uuid::new_v4();

        let mut start_pipe = PipeWriter::inheritable()?;
        let sync_pipe = PipeReader::inheritable()?;

        // Spin up the `conmon` child process.
        let child = Command::new("conmon")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(&["--syslog", "--log-level=debug"])
            .arg("--terminal") // Passes `--console-sock` to `crun`.
            .args(&["--cid", id])
            .args(&["--cuuid", &container_uuid.to_string()])
            .args(&["--name", id])
            .args(&["--runtime", "/usr/bin/crun"])
            .args(&["--runtime-arg", "--rootless=true"])
            .args(&["--bundle", &root.bundle_dir.display().to_string()])
            .args(&["--exit-dir", &root.exits_dir.display().to_string()])
            .args(&["--log-path", &root.log_file.display().to_string()])
            .args(&["--container-pidfile", &root.pid_file.display().to_string()])
            .args(&["--socket-dir-path", &root.base_dir().display().to_string()])
            .env(
                "XDG_RUNTIME_DIR",
                std::env::var_os("XDG_RUNTIME_DIR").unwrap(),
            )
            .inherit_oci_pipes(&start_pipe, &sync_pipe)
            .spawn()?;

        println!(
            "spawned conmon for {}, writing start byte...",
            root.base_dir().display()
        );

        // Write a null byte to `start_pipe`, signalling to start setup.
        if let Err(e) = start_pipe.write_all(&[0u8]).await {
            let output = child.wait_with_output().await?;
            if output.status.success() {
                return Err(anyhow!("failed to write start signal to `conmon`: {}", e));
            } else {
                let stderr = String::from_utf8(output.stderr)?;
                return Err(anyhow!(
                    "failed to write start signal to `conmon` ({}), exited with non-zero status: [{}]",
                    e,
                    stderr
                ));
            }
        }

        println!("wrote start byte, waiting for `conmon` to fork and exec...");

        // Wait for initial setup to complete.
        let output = child.wait_with_output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(anyhow!(
                "failed to create container, `conmon` exited with non-zero status: [{}]",
                stderr
            ));
        }

        println!("reading PID from `conmon`...");

        // Wait to get container PID from `conmon`.
        let mut sync_pipe = BufReader::new(sync_pipe);
        let mut line = String::new();
        sync_pipe.read_line(&mut line).await?;
        let pid = match serde_json::from_str(&line)? {
            SyncInfo::Ok { pid } => pid,
            SyncInfo::Err { pid, message } => {
                return Err(anyhow!(
                    "failed to create container, `sync_pipe` returned {}: [{}]",
                    pid,
                    message
                ))
            }
        };

        println!("received PID {}, connecting to console socket...", pid);

        // Setup is complete, so connect to the console socket.
        let console_sock = UnixSeqpacket::connect(
            root.base_dir()
                .join(container_uuid.to_string())
                .join("attach"),
        )
        .await?;

        println!("connected to console socket!");

        Ok(Container {
            name: id.to_string(),
            uuid: container_uuid,
            pid,
            console_sock,
            sync_pipe,
            runtime_dir: root,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
    }
}
