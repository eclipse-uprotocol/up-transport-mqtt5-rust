# Eclipse uProtocol MQTT 5 Transport Library for Rust

This library provides a Rust based implementation of the [MQTT 5 uProtocol Transport v1.6.0-alpha.5](https://github.com/eclipse-uprotocol/up-spec/blob/v1.6.0-alpha.5/up-l1/mqtt_5.adoc).

## Getting Started

### Clone the Repository

```sh
git clone --recurse-submodules git@github.com:eclipse-uprotocol/up-rust
```

The `--recurse-submodules` parameter is important to make sure that the git submodule referring to the uProtocol specification is being initialized in the workspace. The files contained in that submodule define uProtocol's behavior and are used to trace requirements to implementation and test as part of CI workflows.
If the repository has already been cloned without the parameter, the submodule can be initialized manually using `git submodule update --init --recursive`.

In order to make sure that you pull in any subsequent changes made to submodules from upstream, you need to use

```sh
git pull --recurse-submodules
```

If you want to make Git always pull with `--recurse-submodules`, you can set the configuration option *submodule.recurse* to `true` (this works for git pull since Git 2.15). This option will make Git use the `--recurse-submodules` flag for all commands that support it (except *clone*).

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

The Subscriber supports configuration via command line options and/or environment variables:

```bash
cargo run --example subscriber_example -- --help
```

Run the Subscriber with options appropriate for your MQTT broker. When using the Mosquitto broker started via Docker Compose, then the defaults should work:

```bash
cargo run --example subscriber_example
```

3. Run the Publisher

The Publisher supports configuration via command line options and/or environment variables:

```bash
cargo run --example publisher_example -- --help
```

Run the Publisher with options appropriate for your MQTT broker. When using the Mosquitto broker started via Docker Compose, then the defaults should work:

```bash
cargo run --example publisher_example
```

## Using the Library

Most developers will want to create an instance of the *Mqtt5Transport* struct and use it with the Communication Level API and its default implementation
which are provided by the *up-rust* library.

The libraries need to be added to the `[dependencies]` section of the `Cargo.toml` file:

```toml
[dependencies]
up-rust = { version = "0.6" }
up-transport-mqtt5 = { version = "0.3" }
```

Please refer to the [publisher_example](/examples/publisher_example.rs) and [subscriber_example](/examples/subscriber_example.rs) to see how to initialize and use the transport.

The library contains the following modules:

| Module    | uProtocol Specification                                                                                 | Purpose                                                                                                   |
| --------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/v1.6.0-alpha.5/up-l1/README.adoc) | Implementation of MQTT5 uTransport client used for bidirectional point-2-point communication between uEs. |

### Supported Message Priority Levels
`uman~utransport-send-ignore-priority~1`

The `Mqtt5Transport::send` function always uses a standard MQTT 5 PUBLISH packet to transfer the message to the MQTT broker regardless of the service class set on a uProtocol message.

Covers:
- req~utransport-send-qos-mapping~1

### Supported Message Delivery Methods
`uman~utransport-supported-message-delivery~1`

The MQTT 5 transport provided by this crate supports the [push delivery method](https://github.com/eclipse-uprotocol/up-spec/blob/v1.6.0-alpha.5/up-l1/README.adoc#5-message-delivery) only.
The `Mqtt5Transport::receive` function therefore always returns `UCode::UNIMPLEMENTED`.

Covers:
- `req~utransport-delivery-methods~1`

### Maximum number of listeners
`uman~max-listeners-configuration~1`

The MQTT 5 transport provided by this crate can be configured with the maximum number of filter patterns that listeners can be registered for by means of the `MqttTransportOptions` struct that is being passed into `Mqtt5Transport::new`.
Please refer to the [API Documentation](https://docs.rs/up-transport-mqtt5/) for details.

Covers:
- `req~utransport-registerlistener-max-listeners~1`

## Design

### Message Priority Mapping
`dsn~utransport-send-ignore-priority~1`

The MQTT 5 standard does not define any mechanism for message prioritization. All messages are being processed with the same priority.
Consequently, the MQTT 5 transport provided by this crate simply ignores the service class set on a uProtocol message being sent.

Covers:
- `req~utransport-send-qos-mapping~1`

### Message Delivery
`dsn~utransport-supported-message-delivery~1`

All messages are being received by means of subscribing to relevant topics on the MQTT broker and delivering the messages to listeners that have been registered via `up_rust::UTransport::register_listener`.
The transport spawns a dedicated [tokio task](https://tokio.rs/tokio/tutorial/spawning#tasks) that listens for incoming messages and dispatches them to the registered listeners on the same thread that the message handling task runs on.

Rationale:
The MQTT 5 standard does not provide means to poll the broker for messages but only supports the push model by means of clients subscribing to topic filters.

The transport requires a tokio runtime to execute but does not make any implicit assumption regarding the availability and size of thread pools. This provides for flexibility regarding the environment that the transport can be deployed to but also requires application developers to take care when implementing message listeners, making sure to not block the transport's incoming message handler when processing a dispatched message.

Covers:
- `req~utransport-delivery-methods~1`

Needs: impl, utest
