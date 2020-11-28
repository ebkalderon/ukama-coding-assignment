//! A simple and lightweight container engine implementation.

#![deny(missing_debug_implementations)]

pub use self::container::{State, Status};

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::anyhow;
use dashmap::DashMap;
use fallible_collections::tryformat;
use tracing::{debug, info};
use warp::{Filter, Reply};

use self::container::Container;
use self::image::OciImage;

mod container;
mod image;
mod pipe;
mod rest;

/// The container engine service.
///
/// Note that containers are kept in temporary directories and will be cleaned up automatically
/// once this object is dropped.
#[derive(Clone, Debug)]
pub struct Engine {
    containers: Arc<DashMap<String, Container>>,
}

impl Engine {
    /// Creates a new empty container engine.
    pub fn new() -> Self {
        let containers = Arc::new(DashMap::new());
        let running = containers.clone();

        tokio::spawn(async move {
            if let Ok(_) = tokio::signal::ctrl_c().await {
                running.clear();
                std::process::exit(130);
            }
        });

        Engine { containers }
    }

    /// Fetches an OCI container with the bearing the given `name[:tag]` combination from Docker
    /// Hub, unpacks the bundle into a temporary directory, and starts it.
    ///
    /// This method is idempotent and does nothing if `container_name` already exists.
    ///
    /// Returns `Err` if fetching, unpacking, or creating the container failed, an I/O error
    /// occurred, or if an out-of-memory error was encountered.
    pub async fn create(&self, container_name: &str) -> anyhow::Result<()> {
        if self.containers.contains_key(container_name) {
            debug!("container {} already exists, skipping", container_name);
            return Ok(());
        }

        let fetched_image = OciImage::fetch_from_docker_hub(container_name).await?;
        let runtime_dir = fetched_image.unpack().await?;
        let container = Container::create(container_name, runtime_dir).await?;
        container.start().await?;

        let id = tryformat!(64, "{}", container_name).map_err(|e| anyhow!("OOM error: {:?}", e))?;
        self.containers.insert(id, container);

        Ok(())
    }

    /// Retrieves the current state of a container identified by `name[:tag]` and returns it.
    ///
    /// Returns `Err` if the container does not exist, an I/O error occurred, or if an
    /// out-of-memory error was encountered.
    pub async fn state(&self, container_name: &str) -> anyhow::Result<State> {
        match self.containers.get(container_name) {
            Some(container) => container.state().await,
            None => return Err(anyhow!("container `{}` does not exist", container_name)),
        }
    }

    /// Pauses the execution of a container identified by `name[:tag]` if it is running.
    ///
    /// This method is idempotent and does nothing if the container is already paused.
    ///
    /// Returns `Err` if the container does not exist, an I/O error occurred, or if an
    /// out-of-memory error was encountered.
    pub async fn pause(&self, container_name: &str) -> anyhow::Result<()> {
        match self.containers.get(container_name) {
            Some(container) => container.pause().await,
            None => return Err(anyhow!("container `{}` does not exist", container_name)),
        }
    }

    /// Resumes the execution of a container identified by `name[:tag]` if it is paused.
    ///
    /// This method is idempotent and does nothing if the container is already running.
    ///
    /// Returns `Err` if the container does not exist, an I/O error occurred, or if an
    /// out-of-memory error was encountered.
    pub async fn resume(&self, container_name: &str) -> anyhow::Result<()> {
        match self.containers.get(container_name) {
            Some(container) => container.resume().await,
            None => return Err(anyhow!("container `{}` does not exist", container_name)),
        }
    }

    /// Kills and deletes the container identified by `name[:tag]`.
    ///
    /// Returns `Err` if the container does not exist, an I/O error occurred, or if an
    /// out-of-memory error was encountered.
    pub async fn delete(&self, container_name: &str) -> anyhow::Result<()> {
        match self.containers.remove(container_name) {
            Some((_, container)) => container.delete().await,
            None => return Err(anyhow!("container `{}` does not exist", container_name)),
        }
    }

    /// Serves the container engine as a REST API over the given TCP socket address `addr`.
    ///
    /// # Endpoints
    ///
    /// HTTP Route                      | Request body             | Description
    /// --------------------------------|--------------------------|-------------------------------
    /// `PUT /containers/<name>`        |                          | Fetch/create container
    /// `GET /containers/<name>`        |                          | Get container status as JSON
    /// `DELETE /containers/<name>`     |                          | Delete container
    /// `PUT /containers/<name>/status` | `{ "state": "paused" }`  | Pause container execution
    /// `PUT /containers/<name>/status` | `{ "state": "running" }` | Resume container execution
    #[inline]
    pub async fn serve<A: Into<SocketAddr>>(self, addr: A) {
        let socket_addr = addr.into();
        info!("serving container engine on TCP socket: {}", socket_addr);
        warp::serve(self.into_filter()).run(socket_addr).await
    }

    /// Returns the REST API as a bare [`warp`](https://docs.rs/warp) filter for serving on other
    /// transports besides TCP.
    #[inline]
    pub fn into_filter(self) -> impl Filter<Extract = impl Reply> + Clone + 'static {
        rest::to_filter(self)
    }
}
