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

use std::{future::Future, io::Write, sync::Arc};
use tempfile::TempPath;
use testcontainers::{
    core::{Mount, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use up_rust::UStatus;
use up_transport_mqtt5::{Mqtt5Transport, Mqtt5TransportOptions, MqttClientOptions, TransportMode};

const MOSQUITTO_CONTAINER_PORT: u16 = 1883;

/// Gets a future that completes once the given transport's connection to the MQTT broker has been (re-)established.
///
/// This function is useful for tests that need to wait for the MQTT connection to be ready before proceeding.
/// However, the future being returned is constantly polling the connection status,
/// which may not be the most efficient way to wait for the connection.
pub fn connection_established(transport: Arc<Mqtt5Transport>) -> impl Future<Output = ()> {
    std::future::poll_fn(move |cx| {
        if transport.is_connected() {
            std::task::Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
}

pub async fn create_up_transport_mqtt<S: Into<String>>(
    authority_name: S,
    host_broker_port: u16,
    mqtt_client_id: Option<S>,
) -> Result<Mqtt5Transport, UStatus> {
    let mqtt_client_options = MqttClientOptions {
        // tcp or ssl
        // https://docs.rs/paho-mqtt/latest/paho_mqtt/create_options/struct.CreateOptionsBuilder.html#method.server_uri
        broker_uri: format!("tcp://localhost:{host_broker_port}"),
        clean_start: false,
        client_id: mqtt_client_id.map(|id| id.into()),
        max_buffered_messages: 100,
        session_expiry_interval: 3600,
        ssl_options: None,
        username: None,
        password: None,
    };
    let options = Mqtt5TransportOptions {
        mode: TransportMode::InVehicle,
        max_filters: 10,
        max_listeners_per_filter: 5,
        mqtt_client_options,
    };

    Mqtt5Transport::new(options, authority_name).await
}

pub(crate) struct MosquittoBroker {
    _broker: ContainerAsync<GenericImage>,
    port: u16,
    _config_path: TempPath,
}

impl MosquittoBroker {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }
    pub(crate) async fn start(&self) -> Result<(), String> {
        self._broker
            .start()
            .await
            .map_err(|e| format!("Failed to start Mosquitto: {e}"))
    }
    pub(crate) async fn stop(&self) -> Result<(), String> {
        self._broker
            .stop()
            .await
            .map_err(|e| format!("Failed to stop Mosquitto: {e}"))
    }
}

/// Starts an Eclipse Mosquitto Docker container with given configuration options.
///
/// # Arguments
/// * `config` - Optional string slice containing additional Mosquitto configuration options.
///
/// # Returns
/// A [MosquittoBroker] instance that represents the running Mosquitto container.
/// The container will be stopped and removed when the returned instance is dropped.
///
/// The given configuration will be appended to the default Mosquitto configuration which is:
/// ```text
/// listener 1883
/// allow_anonymous true
/// ```
///
/// The configuration is written to a temporary file which is bind-mounted into the container.
pub(crate) async fn start_mosquitto(
    config: Option<&str>,
    mapped_port: Option<u16>,
) -> MosquittoBroker {
    let mut mosquitto_config =
        tempfile::NamedTempFile::new().expect("failed to create Mosquitto config file");
    writeln!(mosquitto_config, "listener 1883").expect("failed to write to Mosquitto config file");
    writeln!(mosquitto_config, "allow_anonymous true")
        .expect("failed to write to Mosquitto config file");
    if let Some(configuration) = config {
        writeln!(mosquitto_config, "{configuration}")
            .expect("failed to write to Mosquitto config file");
    }

    let config_path = mosquitto_config.into_temp_path();

    let container_port = MOSQUITTO_CONTAINER_PORT.into();
    let container = GenericImage::new("eclipse-mosquitto", "2.0")
        .with_exposed_port(container_port)
        // mosquitto seems to write to stderr
        .with_wait_for(WaitFor::message_on_stderr(" running"))
        // uncomment next line in order capture Mosquitto log messages
        // .with_log_consumer(
        //     testcontainers::core::logs::consumer::logging_consumer::LoggingConsumer::new(),
        // )
        // use given configuration
        .with_mount(Mount::bind_mount(
            config_path.display().to_string(),
            "/mosquitto/config/mosquitto.conf",
        ))
        .with_mapped_port(mapped_port.unwrap_or_default(), container_port)
        .start()
        .await
        .expect("Failed to start Mosquitto");

    let host_port = container
        .get_host_port_ipv4(MOSQUITTO_CONTAINER_PORT)
        .await
        .expect("MQTT port {MOSQUITTO_CONTAINER_PORT} not exposed");

    MosquittoBroker {
        _broker: container,
        port: host_port,
        _config_path: config_path,
    }
}
