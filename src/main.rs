//! Entry point for the application.

use std::net::SocketAddr;

use argh::FromArgs;
use tracing_subscriber::{util::SubscriberInitExt, EnvFilter};

/// Lightweight OCI container engine with REST API.
#[derive(FromArgs)]
struct Opt {
    /// TCP port to listen on [default: 8080]
    #[argh(option, short = 'p', default = "8080")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .finish()
        .try_init()?;

    let Opt { port } = argh::from_env();
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    light_containerd::Engine::new().serve(addr).await;

    Ok(())
}
