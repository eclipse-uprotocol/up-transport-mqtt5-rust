# Eclipse uProtocol Rust MQTT5 Client

## Overview

This library implements a uTransport client for MQTT5 in Rust following the uProtocol [uTransport Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc).

## Getting Started

### Building the Library

To build the library, run `cargo build` in the project root directory. Tests can be run with `cargo test`. This library leverages the [up-rust](https://github.com/eclipse-uprotocol/up-rust/tree/main) library for data types and models specified by uProtocol.

### Using the Library

The library contains the following modules:

Package | [uProtocol spec](https://github.com/eclipse-uprotocol/uprotocol-spec) | Purpose
---|---|---
transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc) | Implementation of MQTT5 uTransport client used for bidirectional point-2-point communication between uEs.
