use std::str;

use clap::Parser;
use log;
use toml;

use reticulum::identity::PrivateIdentity;
use reticulum::iface::udp::UdpInterface;
use reticulum::transport::{Transport, TransportConfig};

use rns_mavlink;

const CONFIG_PATH: &str = "Gc.toml";

/// Command line arguments
#[derive(Parser)]
#[clap(name = "Rns-Mavlink Ground Control Bridge", version)]
pub struct Command {
  #[clap(short = 'p', help = "Reticulum UDP listen port number")]
  pub port: u16,
  #[clap(short = 'f', help = "Reticulum UDP forward link address")]
  pub forward: std::net::SocketAddr
}

#[tokio::main]
async fn main() {
  // parse command line args
  let cmd = Command::parse();
  // load config
  let config: rns_mavlink::GcConfig = {
    use std::io::Read;
    let mut s = String::new();
    let mut f = std::fs::File::open(CONFIG_PATH).unwrap();
    assert!(f.read_to_string(&mut s).unwrap() > 0);
    toml::from_str(&s).unwrap()
  };
  // init logging
  env_logger::Builder::from_env(env_logger::Env::default()
    .default_filter_or(&config.log_level)).init();
  log::info!("gc start with RNS port {} and forward node {}", cmd.port, cmd.forward);
  // mavlink bridge
  let gc = rns_mavlink::Gc::new(config);
  // start reticulum
  log::info!("starting reticulum");
  let id = PrivateIdentity::new_from_name("mavlink-rns-gc");
  let transport = Transport::new(TransportConfig::new("gc", &id, true));
  let _ = transport.iface_manager().lock().await.spawn(
    UdpInterface::new(format!("0.0.0.0:{}", cmd.port), Some(cmd.forward.to_string())),
    UdpInterface::spawn);
  if let Err(err) = gc.run(transport, id).await {
    log::error!("gc bridge exited with error: {:?}", err);
  } else {
    log::info!("gc exit");
  }
}
