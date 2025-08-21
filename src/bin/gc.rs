use std::str;
use std::sync::Arc;
use std::time;

use log;
use tokio::net::UdpSocket;

use reticulum::destination::{DestinationName, SingleInputDestination};
use reticulum::destination::link::{Link, LinkEvent, LinkId};
use reticulum::identity::PrivateIdentity;
use reticulum::iface::tcp_server::TcpServer;
use reticulum::transport::{Transport, TransportConfig};
use reticulum::hash::AddressHash;

pub async fn run(mut transport: Transport, id: PrivateIdentity) {
  let in_destination = transport
    .add_destination(id, DestinationName::new("mavlink_rns", "server")).await;
  let in_destination_hash = in_destination.lock().await.desc.address_hash;
  log::info!("created in destination: {}", in_destination_hash);
  // send announces
  let announce_loop = async || loop {
    transport.send_announce(&in_destination, None).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
  };
  let link_id: Arc<tokio::sync::Mutex<Option<LinkId>>> = Arc::new(tokio::sync::Mutex::new(None));
  // socket loop
  let socket_loop = async || {
    loop {
      log::warn!("TODO: socket listen to QGC");
      tokio::time::sleep(time::Duration::from_secs(1)).await;
    }
  };
  // upstream link data
  let link_loop = async || {
    // TODO: in link address is the server, can we check the upstream address?
    let _client_destination =
      match AddressHash::new_from_hex_string("a05700f0593d63a92324bcfdcf14e6f4") {
        Ok(dest) => dest,
        Err(err) => {
          log::error!("error parsing client destination hash: {err:?}");
          return
        }
      };
    let mut in_link_events = transport.in_link_events();
    while let Ok(link_event) = in_link_events.recv().await {
      match link_event.event {
        LinkEvent::Data(payload) => if link_event.address_hash == in_destination_hash {
          log::trace!("link {} payload ({})", link_event.id, payload.len());
          log::warn!("TODO: send to QGC port 14550");
          /*
          match socket.send(payload.as_slice()).await {
            Ok(n) => log::trace!("tun sent {n} bytes"),
            Err(err) => {
              log::error!("tun error sending bytes: {err:?}");
              break
            }
          }
          */
        }
        LinkEvent::Activated => if link_event.address_hash == in_destination_hash {
          log::debug!("link activated {}", link_event.id);
          let mut link_id = link_id.lock().await;
          *link_id = Some(link_event.id);
        }
        LinkEvent::Closed => if link_event.address_hash == in_destination_hash {
          log::debug!("link closed {}", link_event.id)
        }
      }
    }
  };
  tokio::select!{
    _ = announce_loop() => log::info!("announce loop exited: shutting down"),
    _ = socket_loop() => log::info!("tun loop exited: shutting down"),
    _ = link_loop() => log::info!("link loop exited: shutting down"),
    _ = tokio::signal::ctrl_c() => log::info!("got ctrl-c: shutting down")
  }
}

#[tokio::main]
async fn main() {
  // init logging
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("TRACE")).init();
  // start reticulum
  log::info!("starting reticulum");
  let id = PrivateIdentity::new_from_name("mavlink-rns-server");
  let transport = Transport::new(TransportConfig::new("server", &id, true));
  let _ = transport.iface_manager().lock().await.spawn(
    TcpServer::new(format!("0.0.0.0:{}", 4242), transport.iface_manager()),
    TcpServer::spawn,
  );
  run(transport, id).await
}
