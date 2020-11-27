//! Entry point for the application.

use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr: SocketAddr = "127.0.0.1:8080".parse()?;
    light_containerd::Engine::new().serve(addr).await;
    Ok(())
}
