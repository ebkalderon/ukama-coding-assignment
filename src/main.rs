use std::sync::Arc;

use anyhow::anyhow;
use dashmap::DashMap;
use fallible_collections::FallibleArc;

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
    pub async fn new() -> anyhow::Result<Self> {
        let containers = Arc::try_new(DashMap::new()).map_err(|e| anyhow!("OOM error: {:?}", e))?;
        let running = containers.clone();

        // TODO: Find a way to avoid using `tokio::spawn()` and convert to `join!()` instead.
        tokio::spawn(async move {
            if let Ok(_) = tokio::signal::ctrl_c().await {
                running.clear();
                std::process::exit(130);
            }
        });

        Ok(Engine { containers })
    }

    pub async fn fetch(&mut self, container_name: &str) -> anyhow::Result<()> {
        eprintln!("fetching from dockerhub...");
        let fetched_image = OciImage::fetch_from_docker_hub(container_name).await?;
        eprintln!("fetched from dockerhub, unpacking...");
        let runtime_dir = fetched_image.unpack().await?;
        eprintln!("unpacked, creating container...");
        let container = Container::create(container_name, runtime_dir).await?;
        self.containers.insert(container_name.into(), container);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // TODO: Use `warp` to host REST endpoints.
    let mut engine = Engine::new().await?;
    engine.fetch("busybox").await?;
    tokio::time::sleep(std::time::Duration::from_secs(1000)).await;
    Ok(())
}
