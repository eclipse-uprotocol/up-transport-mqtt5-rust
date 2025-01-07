# Eclipse uProtocol MQTT 5 Transport Library for Rust

## Overview

This library provides a Rust based implementation of the [MQTT 5 uProtocol Transport v1.6.0-alpha.4](https://github.com/eclipse-uprotocol/up-spec/blob/v1.6.0-alpha.4/up-l1/mqtt_5.adoc).

## Getting Started

### Building the Library

To build the library, run `cargo build` in the project root directory.

### Running the Tests

To run the tests from the repo root directory, run
```bash
cargo test
```

### Running the Examples

The example shows how the transport can be used to publish uProtocol messages from one uEntity and consume these messages on another uEntity.

1. Start the Eclipse Mosquitto MQTT broker using Docker Compose:

```bash
docker compose -f tests/mosquitto/docker-compose.yaml up --detach
```

2. Run the Subscriber

```bash
cargo run --example subscriber_example
```

3. Run the Publisher

```bash
cargo run --example publisher_example
```

### Using the Library

The library contains the following modules:

| Module    | uProtocol Specification                                                                                 | Purpose                                                                                                   |
| --------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc) | Implementation of MQTT5 uTransport client used for bidirectional point-2-point communication between uEs. |

Please refer to the [publisher_example](/examples/publisher_example.rs) and [subscriber_example](/examples/subscriber_example.rs) to see how to initialize and use the transport.
