use dashmap::DashMap;

use self::container::Container;
use self::image::OciImage;

mod container;
mod image;
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
    let mut engine = Engine::new();
    engine.fetch("busybox").await?;
    tokio::time::sleep(std::time::Duration::from_secs(1000)).await;
    Ok(())
}
