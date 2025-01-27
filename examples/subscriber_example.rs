/********************************************************************************
 * Copyright (c) 2023 Contributors to the Eclipse Foundation
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

use std::{
    str::{self, FromStr},
    sync::Arc,
    thread,
};

use async_trait::async_trait;
use clap::{command, Parser};
use log::{error, info};
use up_rust::{UListener, UMessage, UStatus, UTransport, UUri};
use up_transport_mqtt5::{Mqtt5Transport, MqttClientOptions, TransportMode};

struct LoggingListener {}

#[async_trait]
impl UListener for LoggingListener {
    async fn on_receive(&self, message: UMessage) {
        let msg_payload = message.payload.unwrap();
        let msg_str: &str = str::from_utf8(&msg_payload).unwrap();
        info!("Received message: {msg_str}");
    }
}

/// Consumes messages from topics matching a given filter using the MQTT 5 transport.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Command {
    /// The uProtocol filter URI to consume messages for.
    #[arg(value_name = "URI", env = "TOPIC_FILTER", default_value = "up://Vehicle_B/FFFFFFFF/FF/FFFF", value_parser = UUri::from_str)]
    topic_filter: UUri,
    #[command(flatten)]
    mqtt_client_options: MqttClientOptions,
}

#[tokio::main]
async fn main() -> Result<(), UStatus> {
    env_logger::init();

    let command = Command::parse();
    let authority = command.topic_filter.authority_name.clone();
    let client = Mqtt5Transport::new(
        TransportMode::InVehicle,
        command.mqtt_client_options,
        authority,
    )
    .await?;

    backoff::future::retry(backoff::ExponentialBackoff::default(), || async {
        info!("Connecting to broker...");
        Ok(client
            .connect()
            .await
            .inspect_err(|err| error!("Connection attempt failed: {err}"))?)
    })
    .await?;

    let listener = Arc::new(LoggingListener {});

    info!(
        "Subscribing to topic: {}",
        command.topic_filter.to_uri(true)
    );

    client
        .register_listener(&command.topic_filter, None, listener.clone())
        .await?;
    thread::park();
    Ok(())
}
