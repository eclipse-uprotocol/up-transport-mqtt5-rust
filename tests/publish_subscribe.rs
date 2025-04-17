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

use std::{str::FromStr, sync::Arc, time::Duration};

use up_rust::{MockUListener, UMessageBuilder, UTransport, UUri};

mod common;

#[tokio::test]
#[cfg_attr(not(docker_available), ignore)]
// This test requires Docker to run the Mosquitto MQTT broker.
async fn test_publish_and_subscribe() {
    env_logger::init();

    // fixture
    let (_mosquitto, broker_port) = common::start_mosquitto().await;

    let payload = "test_payload";
    let expected_payload = payload.to_owned();
    let message_received = Arc::new(tokio::sync::Notify::new());
    let message_received_clone = message_received.clone();
    let mut listener = MockUListener::new();
    listener.expect_on_receive().once().return_once(move |msg| {
        assert_eq!(msg.payload, Some(expected_payload.into()));
        message_received_clone.notify_one();
    });

    let subscriber = common::create_up_transport_mqtt("Subscriber", broker_port)
        .await
        .expect("failed to create transport at receiving end");
    subscriber
        .connect()
        .await
        .expect("failed to connect subscriber to broker");
    let source_filter =
        UUri::from_str("//Publisher/A8000/2/FFFF").expect("Failed to create source filter");
    subscriber
        .register_listener(&source_filter, None, Arc::new(listener))
        .await
        .unwrap();

    let publisher = common::create_up_transport_mqtt("Publisher", broker_port)
        .await
        .expect("failed to create transport at sending end");
    publisher
        .connect()
        .await
        .expect("failed to connect publisher to broker");

    let source = UUri::from_str("//Publisher/A8000/2/8A50").unwrap();
    let message_to_send = UMessageBuilder::publish(source)
        .build_with_payload(payload, up_rust::UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
        .unwrap();
    publisher
        .send(message_to_send)
        .await
        .expect("failed to publish message");

    tokio::time::timeout(Duration::from_millis(1000), message_received.notified())
        .await
        .expect("did not receive published message before timeout");
}
