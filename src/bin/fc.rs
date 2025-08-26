use std::sync::Arc;
use std::time;

use log;
use tokio_serial;

use reticulum::destination::{DestinationName, SingleInputDestination};
use reticulum::destination::link::{Link, LinkEvent};
use reticulum::identity::PrivateIdentity;
use reticulum::iface::udp::UdpInterface;
use reticulum::transport::{Transport, TransportConfig};
use reticulum::hash::AddressHash;

pub async fn run(transport: Transport) {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  use tokio_serial::SerialPortBuilderExt;
  let link: Arc<tokio::sync::Mutex<Option<Arc<tokio::sync::Mutex<Link>>>>> =
    Arc::new(tokio::sync::Mutex::new(None));
  let server_destination =
    match AddressHash::new_from_hex_string("5285c511344e3b5b3a765229554e4da9") {
      Ok(dest) => dest,
      Err(err) => {
        log::error!("error parsing server destination hash: {err:?}");
        return
      }
    };
  let port = tokio_serial::new("/dev/ttyACM0", 115200)
    .open_native_async()
    .unwrap();
  let (mut port_reader, mut port_writer) = tokio::io::split(port);
  // set up links
  let link_loop = async || {
    let mut announce_recv = transport.recv_announces().await;
    // TODO: continue looping after link is created?
    while let Ok(announce) = announce_recv.recv().await {
      let destination = announce.destination.lock().await;
      if destination.desc.address_hash == server_destination {
        *link.lock().await = Some(transport.link(destination.desc).await);
      }
    }
  };
  // read serial port and forward to links
  let mut read_port_loop = async || {
    loop {
      if let Some(_link) = link.lock().await.as_ref() {
        log::info!("Reading from serial port ttyACM0...");
        let mut buf = vec![0u8; 2usize.pow(16)];
        loop {
          match port_reader.read(&mut buf).await {
            Ok(n) => {
              log::trace!("Read {n} bytes");

              for data in buf[..n].chunks(reticulum::packet::PACKET_MDU / 2) {
                /*FIXME:debug*/ //log::warn!("SENDING LINK DATA");
                println!("DATA LEN: {}", data.len());
                transport.send_to_all_out_links(data).await;
              }
            }
            Err(e) => {
              log::error!("Error receiving packet: {}", e);
            }
          }
        }
      } else {
        tokio::time::sleep(time::Duration::from_millis(100)).await
      }
    }
  };
  // forward upstream link messages to serial port
  let mut write_port_loop = async || {
    let mut out_link_events = transport.out_link_events();
    while let Ok(link_event) = out_link_events.recv().await {
      match link_event.event {
        LinkEvent::Data(payload) => if link_event.address_hash == server_destination {
          log::trace!("link {} payload ({})", link_event.id, payload.len());
          match port_writer.write_all(payload.as_slice()).await {
            Ok(()) => log::trace!("port sent {} bytes", payload.len()),
            Err(err) => {
              log::error!("port error sending bytes: {err:?}");
              break
            }
          }
        }
        LinkEvent::Activated => if link_event.address_hash == server_destination {
          log::info!("link activated {}", link_event.id);
        }
        LinkEvent::Closed => if link_event.address_hash == server_destination {
          log::warn!("link closed {}", link_event.id);
          let _ = link.lock().await.take();
        }
      }
    }
  };
  // run
  tokio::select!{
    _ = read_port_loop() => log::info!("read port loop exited: shutting down"),
    _ = write_port_loop() => log::info!("write port loop exited: shutting down"),
    _ = link_loop() => log::info!("link loop exited: shutting down"),
    _ = tokio::signal::ctrl_c() => log::info!("got ctrl-c: shutting down")
  }
}

#[tokio::main]
async fn main() {
  // init logging
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("TRACE")).init();
  // start reticulum
  let id = PrivateIdentity::new_from_name("mavlink-rns-fc");
  let transport = Transport::new(TransportConfig::new("fc", &id, true));
  let _ = transport.iface_manager().lock().await.spawn(
    //UdpInterface::new("0.0.0.0:4243", Some("192.168.1.131:4242")),
    UdpInterface::new("0.0.0.0:4243", Some("127.0.0.1:4242")),
    UdpInterface::spawn);
  let destination = SingleInputDestination::new(id, DestinationName::new("mavlink_rns", "client"));
  log::info!("created destination: {}", destination.desc.address_hash);
  run(transport).await
}
