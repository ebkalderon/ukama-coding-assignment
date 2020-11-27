use std::sync::Arc;

use anyhow::anyhow;
use dashmap::DashMap;
use fallible_collections::tryformat;

use self::container::Container;
use self::image::OciImage;

mod container;
mod image;
mod pipe;

#[derive(Debug)]
pub struct Engine {
    containers: Arc<DashMap<String, Container>>,
}

impl Engine {
    pub fn new() -> Self {
        let containers = Arc::new(DashMap::new());
        let running = containers.clone();

        // TODO: Find a way to avoid using `tokio::spawn()` and convert to `join!()` instead.
        tokio::spawn(async move {
            if let Ok(_) = tokio::signal::ctrl_c().await {
                running.clear();
                std::process::exit(130);
            }
        });

        Engine { containers }
    }

    pub async fn create(&self, container_name: &str) -> anyhow::Result<()> {
        if self.containers.contains_key(container_name) {
            return Ok(());
        }

        eprintln!("fetching from dockerhub...");
        let fetched_image = OciImage::fetch_from_docker_hub(container_name).await?;

        eprintln!("fetched from dockerhub, unpacking...");
        let runtime_dir = fetched_image.unpack().await?;

        eprintln!("unpacked, creating container...");
        let container = Container::create(container_name, runtime_dir).await?;
        container.start().await?;
        eprintln!("started container");

        let id = tryformat!(64, "{}", container_name).map_err(|e| anyhow!("OOM error: {:?}", e))?;
        self.containers.insert(id, container);

        Ok(())
    }

    pub async fn pause(&self, container_name: &str) -> anyhow::Result<()> {
        match self.containers.get(container_name) {
            Some(container) => container.pause().await,
            None => return Err(anyhow!("container `{}` does not exist", container_name)),
        }
    }

    pub async fn resume(&self, container_name: &str) -> anyhow::Result<()> {
        match self.containers.get(container_name) {
            Some(container) => container.resume().await,
            None => return Err(anyhow!("container `{}` does not exist", container_name)),
        }
    }

    pub async fn delete(&self, container_name: &str) -> anyhow::Result<()> {
        match self.containers.remove(container_name) {
            Some((_, container)) => container.delete().await,
            None => return Err(anyhow!("container `{}` does not exist", container_name)),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // TODO: Use `warp` to host REST endpoints.
    let engine = Engine::new();
    engine.create("busybox").await?;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    engine.pause("busybox").await?;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    engine.resume("busybox").await?;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    Ok(())
}
