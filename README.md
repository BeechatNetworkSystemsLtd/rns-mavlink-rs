# `rns-mavlink`

> Reticulum-Mavlink bridge

Bridges a flight controller connectd via serial port to a QGroundControl ground station
over Reticulum mesh network.

## Building and running

Ground Control and Flight Conroller binaries require ports and forward links to be
specified as command-line arguments:

```
# ground control
cargo run --bin gc -- -p 4242 -f 127.0.0.1:4243
# flight controller
cargo run --bin fc -- -p 4243 -f 127.0.0.1:4242
```

Additional configuration such as serial device and QGroundControl ports are set in
`Gc.toml` and `Fc.toml` config files.
