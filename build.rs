/********************************************************************************
 * Copyright (c) 2025 Contributors to the Eclipse Foundation
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 ********************************************************************************/
use testcontainers::{runners::SyncRunner, GenericImage};

pub fn main() {
    // Try to pull the Mosquitto MQTT broker container image which will be needed to
    // run integration tests.
    if let Err(err) = GenericImage::new("eclipse-mosquitto", "2.0").pull_image() {
        eprintln!("Docker is not available on build platform: {err}");
    } else {
        // Tests using #[cfg(docker_available)] will only be compiled (and run)
        // if Docker is actually available on the platform (as determined by the
        // testcontainers crate).
        eprintln!("Docker is available on build platform");
        println!("cargo::rustc-cfg=docker_available");
    }
    // we should only need to determine once, if Docker is available
    // triggering re-execution of this check should always be possible
    // by means of doing a cargo clean ...
    println!("cargo::rerun-if-changed=build.rs");
}
