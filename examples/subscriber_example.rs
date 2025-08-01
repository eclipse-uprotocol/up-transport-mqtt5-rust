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
use backon::{ExponentialBuilder, Retryable};
use clap::{command, Parser};
use log::{error, info};
use up_rust::{UListener, UMessage, UStatus, UTransport, UUri};
use up_transport_mqtt5::{Mqtt5Transport, Mqtt5TransportOptions};

struct LoggingListener {}

#[async_trait]
impl UListener for LoggingListener {
    async fn on_receive(&self, message: UMessage) {
        // Make sure to not block the incoming message handler by spawning a new task
        // for processing the message.
        // Note that this does not per se guarantee that the message will be processed
        // on a different thread than the transport's incoming message handler but it
        // does ensure that this function returns quickly, allowing the incoming message
        // handler to proceed as soon as possible.
        tokio::spawn(async move {
            let msg_payload = message.payload.unwrap();
            let msg_str: &str = str::from_utf8(&msg_payload).unwrap();
            info!("Received message: {msg_str}");
            // simulate some time consuming processing
            thread::sleep(std::time::Duration::from_millis(500));
            info!("Finished processing message");
        });
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
    transport_options: Mqtt5TransportOptions,
}

#[tokio::main]
async fn main() -> Result<(), UStatus> {
    env_logger::init();

    let command = Command::parse();
    let authority = command.topic_filter.authority_name.clone();
    let client = Mqtt5Transport::new(command.transport_options, authority).await?;

    (|| {
        info!("Connecting to broker...");
        client.connect()
    })
    .retry(ExponentialBuilder::default())
    .when(|err| {
        error!("Connection attempt failed: {err}");
        true
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
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl-C handler");
    Ok(())
}
