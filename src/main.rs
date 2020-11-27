//! Entry point for the application.

use std::net::SocketAddr;

use argh::FromArgs;

/// Lightweight container engine with REST API.
#[derive(FromArgs)]
struct Opt {
    /// TCP port to listen on [default: 8080]
    #[argh(option, short = 'p', default = "8080")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Opt { port } = argh::from_env();
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    light_containerd::Engine::new().serve(addr).await;
    Ok(())
}
