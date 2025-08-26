use tokio_serial;

#[tokio::main]
async fn main() {
  use tokio::io::AsyncReadExt;
  use tokio_serial::{SerialPort, SerialPortBuilderExt};
  let mut port = tokio_serial::new("/dev/ttyACM0", 115200)
    .open_native_async()
    .unwrap();
  //let _ = port.set_timeout(std::time::Duration::from_secs(1));
  println!("TIMEOUT: {:?}", port.timeout());
  let mut buf = [0u8; 2usize.pow(16)];
  loop {
    match tokio::time::timeout(std::time::Duration::from_millis(100), port.read(&mut buf)).await {
      Ok(Ok(n)) => {
        println!("got {n} bytes");
      }
      Ok(Err(e)) => println!("ERR1: {e:?}"),
      Err(e) => println!("ERR2: {e:?}")
    }
  }
}
