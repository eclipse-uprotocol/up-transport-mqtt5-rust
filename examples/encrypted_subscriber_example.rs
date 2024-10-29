/********************************************************************************
 * Copyright (c) 2024 Contributors to the Eclipse Foundation
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
    env,
    str::{self, FromStr},
    sync::Arc,
};

use async_trait::async_trait;
use env_logger::{Builder, Target};
use log::LevelFilter;
use paho_mqtt::SslOptionsBuilder;
use up_client_mqtt5_rust::{MqttConfig, MqttProtocol, UPClientMqtt, UPClientMqttType};
use up_rust::{UListener, UMessage, UStatus, UTransport, UUri, UUID};

const WILDCARD_ENTITY_ID: u32 = 0x0000_FFFF;
const WILDCARD_ENTITY_VERSION: u32 = 0x0000_00FF;
const WILDCARD_RESOURCE_ID: u32 = 0x0000_FFFF;

struct PrintlnListener {}

#[async_trait]
impl UListener for PrintlnListener {
    async fn on_receive(&self, message: UMessage) {
        let msg_payload = message.payload.unwrap();
        let msg_str: &str = str::from_utf8(&msg_payload).unwrap();
        println!("Received message: {msg_str}");
    }
}

#[tokio::main]
async fn main() -> Result<(), UStatus> {
    Builder::new()
        .target(Target::Stdout) // Logs to stdout
        .filter(None, LevelFilter::Trace) // Default level
        .init();

    // Set the protocol type ("mqtts" for encrypted mqtt)
    let protocol = MqttProtocol::Mqtts;

    // Build the ssl options (only needed if protocol is Mqtts!)
    let ssl_options = Some(
        SslOptionsBuilder::new()
            .key_store(env::var("KEY_STORE").expect("KEY_STORE env variable not found"))
            .expect("Certificate file not found.")
            .private_key_password(
                env::var("PRIVATE_KEY_PW").expect("PRIVATE_KEY_PW env variable not found"),
            )
            .enable_server_cert_auth(false)
            .finalize(),
    );
    // If the mqtt broker has a specific username attached to the ssl certificate, it must be included in the config
    let user_name = env::var("CLIENT_NAME")
        .expect("CLIENT_NAME env variable not found")
        .to_string();

    // Build the configuration for the connection
    let config = MqttConfig {
        mqtt_protocol: protocol,
        mqtt_hostname: env::var("MQTT_HOSTNAME")
            .expect("MQTT_HOSTNAME env variable not found")
            .to_string(),
        mqtt_port: 8883,
        max_buffered_messages: 100,
        max_subscriptions: 100,
        session_expiry_interval: 3600,
        ssl_options,
        username: user_name,
    };

    let client = UPClientMqtt::new(
        config,
        UUID::build(),
        "Vehicle_B".to_string(),
        UPClientMqttType::Device,
    )
    .await?;

    let listener = Arc::new(PrintlnListener {});
    let source_filter = UUri::from_str(&format!(
        "//Vehicle_B/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}"
    ))
    .expect("Failed to create source filter");

    println!("Subscribing to: {}", source_filter.to_uri(false));

    client
        .register_listener(&source_filter, None, listener.clone())
        .await?;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
