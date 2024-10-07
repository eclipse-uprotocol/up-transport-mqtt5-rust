# Eclipse uProtocol Rust MQTT5 Client

## Overview

This library implements a uTransport client for MQTT5 in Rust following the uProtocol [uTransport Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc).

## Getting Started

### Building the Library

To build the library, run `cargo build` in the project root directory. Tests can be run with `cargo test`. This library leverages the [up-rust](https://github.com/eclipse-uprotocol/up-rust/tree/main) library for data types and models specified by uProtocol.

### Running the Tests

To run the tests from the repo root directory, run
```bash
cargo test
```

### Running the Local Examples

First, ensure you have a local MQTT broker running, such as [Mosquitto](https://github.com/eclipse/mosquitto).

Make sure to set all of the required parameters in the .cargo/config.toml:
```toml
KEY_STORE = "not needed"
PRIVATE_KEY_PW = "not needed"
MQTT_HOSTNAME = "localhost for the default mosquitto setup"
MQTT_PORT = "1883 for the default mosquitto setup"
CLIENT_NAME = "not needed"
```

Then start the following two examples from your repo root directory.

```bash
cargo run --example publisher_example
```

```bash
cargo run --example subscriber_example
```

### Running the Examples for Azure

First create the eventgrid:
1. create the eventgrid
2. create a namespace
3. create a topic space with template: d/VehicleA/# for uprotocol topics //VehicleA/*
4. create client

Give the client a certificate:
1. create a ssl certificate
2. add the thumbprint to the client

Give the client access to topics
1. give publish and subscribe rights to the topics

Make sure to set all of the required parameters in the .cargo/config.toml:
```toml
KEY_STORE = "the .pem file location corresponding to the eventgrid clients ssl certificate"
PRIVATE_KEY_PW = "the password to the .pem file"
MQTT_HOSTNAME = "the hostname/ url of the eventgrid"
MQTT_PORT = "8883 for ssl encrypted mqtt"
CLIENT_NAME = "the name of the eventgrid client"
```

Then start the following two examples from your repo root directory.

```bash
cargo run --example publisher_example
```

```bash
cargo run --example subscriber_example
```

This shows an example of a UPMqttClient publishing from one device and a UPMqttClient subscribing to the publishing device to receive data.

### Using the Library

The library contains the following modules:

Package | [uProtocol spec](https://github.com/eclipse-uprotocol/uprotocol-spec) | Purpose
---|---|---
transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc) | Implementation of MQTT5 uTransport client used for bidirectional point-2-point communication between uEs.

Please refer to the [publisher_example](/examples/publisher_example.rs) and [subscriber_example](/examples/subscriber_example.rs) examples to see how to initialize and use the [UPClientMqtt](/src/transport.rs) client.
