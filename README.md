# `rns-mavlink`

> Reticulum-Mavlink bridge

Bridges a flight controller connectd via serial port to a QGroundControl ground station
over Reticulum mesh network.

## Building and running

Ground Control (`gc`) and Flight Conroller (`fc`) binaries require ports and forward
links to be specified as command-line arguments:

```
# ground control
cargo run --bin gc -- -p 4242 -f 127.0.0.1:4243
# flight controller
cargo run --bin fc -- -p 4243 -f 127.0.0.1:4242
```

Additional configuration such as serial device and QGroundControl ports are set in
`Gc.toml` and `Fc.toml` config files.

The `gc` application runs on a system that can communicate with QGroundControl via UDP
(either locally or over internet). Configuration is as follows:

-`qgc_udp_address` -- UDP address:port where QGroundControl is reachable, example:
  `"127.0.0.1:14550"`
-`qgc_reply_port` -- local UDP port where QGroundControl will send replies, example:
  `9999`
-`fc_destination` -- Reticulum address hash of the fc node, example:
  `"db332f13541eb2e4b47d02923fbbcb9a"`

The `fc` application runs on a system that is connected to a flight controller via
USB/serial port. Configuration:

- `serial_port` -- serial port where the flight controller is connected, example:
  `"/dev/ttyACM0"`
- `serial_baud` -- serial port baud rate, example: `115200`
- `gc_destination` -- Reticulum address hash of the gc node, example:
  `"758727c1d044e1fd8a838dc8d1832e95"`

The provided `fc` and `gc` binaries initialize UDP interfaces for next-hop communication
over Reticulum. The `rns_mavlink` library can be used with other Reticulum
configurations by initializing your own `Transport` instance and passing as an argument
to the `Fc` and `Gc` structs when running.
