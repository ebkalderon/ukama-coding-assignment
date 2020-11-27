//! Types for fetching and unpacking OCI images.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::anyhow;
use fallible_collections::{tryformat, vec::TryCollect};
use tempfile::TempDir;
use tokio::process::Command;

const SKOPEO_BIN: &str = "skopeo";
const UMOCI_BIN: &str = "umoci";

/// Represents a fetched OCI image.
#[derive(Debug)]
pub struct OciImage(TempDir);

impl OciImage {
    /// Retrieves an image from Docker Hub with the given spec (either `name` or `name:tag`).
    pub async fn fetch_from_docker_hub(container_spec: &str) -> anyhow::Result<Self> {
        let segments: Vec<_> = container_spec
            .splitn(2, ':')
            .try_collect()
            .map_err(|e| anyhow!("OOM error: {:?}", e))?;

        let (name, tag) = match segments[..] {
            [name] => (name, "latest"),
            [name, tag] => (name, tag),
            _ => return Err(anyhow!("container specification cannot be empty")),
        };

        let src_dir = tempfile::tempdir()?;

        let image_src = tryformat!(64, "docker://docker.io/{}:{}", name, tag)
            .map_err(|e| anyhow!("OOM error: {:?}", e))?;

        let image_dest = tryformat!(256, "oci:{}:{}", src_dir.path().display(), tag)
            .map_err(|e| anyhow!("OOM error: {:?}", e))?;

        let mut fetch_cmd = Command::new(SKOPEO_BIN);
        let output = fetch_cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(&["copy", &image_src, &image_dest])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(anyhow!(
                "failed to fetch container, `{:?}` returned non-zero exit status: [{}]",
                fetch_cmd,
                stderr
            ));
        }

        Ok(OciImage(src_dir))
    }

    /// Unpacks the downloaded image into a runnable form.
    pub async fn unpack(self) -> anyhow::Result<OciBundle> {
        OciBundle::unpack_from(self.0.path()).await
    }
}

/// A directory containing an unpacked OCI image.
///
/// The directory will delete itself automtically when the object is dropped.
#[derive(Debug)]
pub struct OciBundle {
    base_dir: TempDir,
    /// Path to the `bundle` subdirectory, containing the unpacked bundle.
    pub bundle_dir: PathBuf,
    /// Path to the `exits` subdirectory, containing any previous exits recorded by `conmon`.
    pub exits_dir: PathBuf,
    /// Path to the container log file.
    pub log_file: PathBuf,
    /// Path to the running container PID file.
    pub pid_file: PathBuf,
}

impl OciBundle {
    async fn unpack_from(oci_src: &Path) -> anyhow::Result<Self> {
        debug_assert!(oci_src.exists());
        debug_assert!(oci_src.is_dir());

        // Create new base directory and subdirectory paths for unpacked image.
        let base_dir = tempfile::tempdir()?;
        let bundle_dir = base_dir.path().join("bundle");
        let exits_dir = base_dir.path().join("exits");
        let pid_file = base_dir.path().join("container.pid");
        let log_file = base_dir.path().join("container.log");

        let image_flag = tryformat!(256, "--image={}:latest", oci_src.display())
            .map_err(|e| anyhow!("OOM error: {:?}", e))?;

        // Unpack the image into the `bundle` subdirectory.
        let mut unpack_cmd = Command::new(UMOCI_BIN);
        let output = unpack_cmd
            .args(&["unpack", "--rootless"])
            .arg(image_flag)
            .arg(&bundle_dir)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(anyhow!(
                "failed to unpack OCI container, `{:?}` returned non-zero exit status: [{}]",
                unpack_cmd,
                stderr
            ));
        }

        // Create the `exits` subdirectory so it can be used by `conmon` later.
        tokio::fs::create_dir(&exits_dir).await?;

        Ok(OciBundle {
            base_dir,
            bundle_dir,
            exits_dir,
            log_file,
            pid_file,
        })
    }

    /// Returns the base directory path.
    pub(crate) fn base_dir(&self) -> &Path {
        self.base_dir.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUSYBOX_OCI_IMAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/busybox");

    #[tokio::test]
    async fn unpacks_image_correctly() {
        let bundle = OciBundle::unpack_from(Path::new(BUSYBOX_OCI_IMAGE))
            .await
            .expect("failed to unpack bundle");

        assert!(bundle.bundle_dir.exists());
        assert!(bundle.bundle_dir.is_dir());

        let rootfs_dir = bundle.bundle_dir.join("rootfs");
        assert!(rootfs_dir.exists());
        assert!(rootfs_dir.is_dir());

        let config_file = bundle.bundle_dir.join("config.json");
        assert!(config_file.exists());
        assert!(config_file.is_file());

        let umoci_file = bundle.bundle_dir.join("umoci.json");
        assert!(umoci_file.exists());
        assert!(umoci_file.is_file());

        assert!(bundle.exits_dir.exists());
        assert!(bundle.exits_dir.is_dir());

        assert!(!bundle.log_file.exists());
        assert!(!bundle.pid_file.exists());
    }
}
