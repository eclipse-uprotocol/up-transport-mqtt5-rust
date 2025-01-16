# Eclipse uProtocol MQTT 5 Transport Library for Rust

This library provides a Rust based implementation of the [MQTT 5 uProtocol Transport v1.6.0-alpha.4](https://github.com/eclipse-uprotocol/up-spec/blob/v1.6.0-alpha.4/up-l1/mqtt_5.adoc).

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

```bash
cargo run --example subscriber_example
```

3. Run the Publisher

```bash
cargo run --example publisher_example
```

### Using the Library

Most developers will want to create an instance of the *Mqtt5Transport* struct and use it with the Communication Level API and its default implementation
which are provided by the *up-rust* library.

The libraries need to be added to the `[dependencies]` section of the `Cargo.toml` file:

```toml
[dependencies]
up-rust = { version = "0.4" }
up-transport-mqtt5 = { version = "0.2" }
```

Please refer to the [publisher_example](/examples/publisher_example.rs) and [subscriber_example](/examples/subscriber_example.rs) to see how to initialize and use the transport.

The library contains the following modules:

| Module    | uProtocol Specification                                                                                 | Purpose                                                                                                   |
| --------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc) | Implementation of MQTT5 uTransport client used for bidirectional point-2-point communication between uEs. |

