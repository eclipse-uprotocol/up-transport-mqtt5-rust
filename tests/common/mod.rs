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

use testcontainers::{
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use up_rust::UStatus;
use up_transport_mqtt5::{Mqtt5Transport, MqttClientOptions, TransportMode};

pub async fn create_up_transport_mqtt<S: Into<String>>(
    authority_name: S,
    host_broker_port: u16,
) -> Result<Mqtt5Transport, UStatus> {
    let config = MqttClientOptions {
        // tcp or ssl
        // https://docs.rs/paho-mqtt/latest/paho_mqtt/create_options/struct.CreateOptionsBuilder.html#method.server_uri
        broker_uri: format!("tcp://localhost:{host_broker_port}"),
        clean_start: false,
        client_id: None,
        max_buffered_messages: 100,
        max_subscriptions: 100,
        session_expiry_interval: 3600,
        ssl_options: None,
        username: None,
        password: None,
    };

    Mqtt5Transport::new(TransportMode::InVehicle, config, authority_name.into()).await
}
/// Starts a mosquitto docker container and returns the container and the host port.
///
/// The returned [ContainerAsync] will stop and remove the Docker Container
/// when dropped
pub async fn start_mosquitto() -> (ContainerAsync<GenericImage>, u16) {
    const MOSQUITTO_CONTAINER_PORT: u16 = 1883;

    let container = GenericImage::new("eclipse-mosquitto", "2.0.21")
        .with_exposed_port(ContainerPort::Tcp(MOSQUITTO_CONTAINER_PORT))
        // mosquitto seems to write to stderr
        .with_wait_for(WaitFor::message_on_stderr(" running"))
        // use the config for anonymous connects available in mosquitto image
        .with_cmd(["/usr/sbin/mosquitto", "-c", "/mosquitto-no-auth.conf"])
        .start()
        .await
        .expect("Failed to start Mosquitto");

    let host_port = container
        .get_host_port_ipv4(MOSQUITTO_CONTAINER_PORT)
        .await
        .unwrap_or_else(|_| panic!("Port {MOSQUITTO_CONTAINER_PORT} should have been exposed"));

    (container, host_port)
}
