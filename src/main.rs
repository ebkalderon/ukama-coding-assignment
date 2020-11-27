use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::anyhow;
use dashmap::DashMap;
use tempfile::TempDir;
use tokio::process::Command;

use self::container::Container;

mod container;
mod pipe;

#[derive(Debug)]
pub struct Engine {
    containers: DashMap<String, Container>,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            containers: DashMap::new(),
        }
    }

    pub async fn fetch(&mut self, container_name: &str) -> anyhow::Result<()> {
        println!("fetching from dockerhub...");
        let src_dir = fetch_from_docker_hub(container_name).await?;
        println!("fetched from dockerhub, unpacking...");
        let runtime_dir = RuntimeDir::unpack_from(src_dir.path()).await?;
        println!("unpacked, creating container...");
        let container = Container::create(container_name, runtime_dir).await?;
        self.containers.insert(container_name.into(), container);
        Ok(())
    }
}

async fn fetch_from_docker_hub(container_spec: &str) -> anyhow::Result<TempDir> {
    let segments: Vec<_> = container_spec.splitn(2, ':').collect();
    let (name, tag) = match segments[..] {
        [name] => (name, "latest"),
        [name, tag] => (name, tag),
        _ => return Err(anyhow!("container specification cannot be empty")),
    };

    let src_dir = tempfile::tempdir()?;
    let output = Command::new("skopeo")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("copy")
        .arg(format!("docker://docker.io/{}:{}", name, tag))
        .arg(format!("oci:{}:{}", src_dir.path().display(), tag))
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr)?;
        return Err(anyhow!(
            "failed to fetch from Docker Hub, `skopeo` returned non-zero exit status: [{}]",
            stderr
        ));
    }

    Ok(src_dir)
}

#[derive(Debug)]
pub struct RuntimeDir {
    base_dir: TempDir,
    pub bundle_dir: PathBuf,
    pub exits_dir: PathBuf,
    pub log_file: PathBuf,
    pub pid_file: PathBuf,
}

impl RuntimeDir {
    pub async fn unpack_from(oci_src: &Path) -> anyhow::Result<Self> {
        debug_assert!(oci_src.exists());
        debug_assert!(oci_src.is_dir());

        let base_dir = tempfile::tempdir()?;
        let bundle_dir = base_dir.path().join("bundle");
        let exits_dir = base_dir.path().join("exits");
        let pid_file = base_dir.path().join("container.pid");
        let log_file = base_dir.path().join("container.log");

        let output = Command::new("umoci")
            .args(&["unpack", "--rootless"])
            .arg(format!("--image={}:latest", oci_src.display()))
            .arg(&bundle_dir)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(anyhow!(
                "failed to unpack OCI container, `umoci` returned non-zero exit status: [{}]",
                stderr
            ));
        }

        tokio::fs::remove_file(bundle_dir.join("config.json")).await?;

        let output = Command::new("crun")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(&["spec", "--rootless"])
            .current_dir(&bundle_dir)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(anyhow!(
                "failed to generate rootless spec, `crun` returned non-zero exit status: [{}]",
                stderr
            ));
        }

        tokio::fs::create_dir(&exits_dir).await?;

        Ok(RuntimeDir {
            base_dir,
            bundle_dir,
            exits_dir,
            log_file,
            pid_file,
        })
    }

    pub fn base_dir(&self) -> &Path {
        self.base_dir.path()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut engine = Engine::new();
    engine.fetch("busybox").await?;
    tokio::time::sleep(std::time::Duration::from_secs(1000)).await;
    Ok(())
}
