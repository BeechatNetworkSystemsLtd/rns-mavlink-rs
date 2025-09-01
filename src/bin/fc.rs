use clap::Parser;
use log;

use reticulum::destination::{DestinationName, SingleInputDestination};
use reticulum::identity::PrivateIdentity;
use reticulum::iface::udp::UdpInterface;
use reticulum::transport::{Transport, TransportConfig};

use rns_mavlink;

const CONFIG_PATH: &str = "Fc.toml";

/// Command line arguments
#[derive(Parser)]
#[clap(name = "Rns-Mavlink Flight Controller Bridge", version)]
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
  let config: rns_mavlink::FcConfig = {
    use std::io::Read;
    let mut s = String::new();
    let mut f = std::fs::File::open(CONFIG_PATH).unwrap();
    assert!(f.read_to_string(&mut s).unwrap() > 0);
    toml::from_str(&s).unwrap()
  };
  // init logging
  env_logger::Builder::from_env(env_logger::Env::default()
    .default_filter_or(&config.log_level)).init();
  log::info!("fc start with RNS port {} and forward node {}", cmd.port, cmd.forward);
  // mavlink bridge
  let fc = match rns_mavlink::Fc::new(config) {
    Ok(fc) => fc,
    Err(err) => {
      log::error!("error creating fc bridge: {:?}", err);
      std::process::exit(1)
    }
  };
  // start reticulum
  log::info!("starting reticulum");
  let id = PrivateIdentity::new_from_name("mavlink-rns-fc");
  let transport = Transport::new(TransportConfig::new("fc", &id, true));
  let _ = transport.iface_manager().lock().await.spawn(
    UdpInterface::new(format!("0.0.0.0:{}", cmd.port), Some(cmd.forward.to_string())),
    UdpInterface::spawn);
  let destination =
    SingleInputDestination::new(id, DestinationName::new("rns_mavlink", "fc"));
  log::info!("created destination: {}", destination.desc.address_hash);
  // run
  if let Err(err) = fc.run(transport).await {
    log::error!("fc bridge exited with error: {:?}", err);
  } else {
    log::info!("fc exit");
  }
}
