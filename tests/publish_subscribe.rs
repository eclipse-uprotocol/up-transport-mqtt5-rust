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

use log::debug;
use serial_test::serial;
use up_rust::{MockUListener, UMessageBuilder, UTransport, UUri};

mod common;

const MOSQUITTO_CONFIG_W_PERSISTENCE: &str = r#"
persistence true
persistence_location /mosquitto/data/
"#;

#[test_case::test_case(Some(MOSQUITTO_CONFIG_W_PERSISTENCE); "for Mosquitto with persistence")]
#[test_case::test_case(None; "for Mosquitto without persistence")]
#[tokio::test]
#[serial]
#[cfg_attr(not(docker_available), ignore)]
// This test requires Docker to run the Mosquitto MQTT broker.
async fn test_publish_and_subscribe_succeeds_after_reconnect(mosquitto_config: Option<&str>) {
    let _ = env_logger::try_init();

    // fixture
    let mosquitto = common::start_mosquitto(mosquitto_config, Some(15000)).await;
    let topic = UUri::from_str("//publisher/A8000/2/8A50").expect("invalid topic URI");
    let message_to_send = UMessageBuilder::publish(topic)
        .build_with_payload(
            "test_payload",
            up_rust::UPayloadFormat::UPAYLOAD_FORMAT_TEXT,
        )
        .expect("Failed to build message");
    let cloned_message = message_to_send.clone();

    let message_received = Arc::new(tokio::sync::Notify::new());
    let message_received_barrier = message_received.clone();
    let mut listener = MockUListener::new();
    listener.expect_on_receive().returning(move |msg| {
        debug!("handling message in listener");
        assert_eq!(msg, cloned_message);
        message_received.notify_one();
    });

    let subscriber = common::create_up_transport_mqtt(
        "subscriber",
        mosquitto.port(),
        Some("subscriber_client_id"),
    )
    .await
    .map(Arc::new)
    .expect("failed to create transport at receiving end");
    subscriber
        .connect()
        .await
        .expect("failed to connect subscriber to broker");
    let source_filter =
        UUri::from_str("//publisher/A8000/2/FFFF").expect("Failed to create source filter");
    subscriber
        .register_listener(&source_filter, None, Arc::new(listener))
        .await
        .expect("failed to register listener");

    let publisher = common::create_up_transport_mqtt(
        "publisher",
        mosquitto.port(),
        Some("publisher_client_id"),
    )
    .await
    .map(Arc::new)
    .expect("failed to create transport at sending end");
    publisher
        .connect()
        .await
        .expect("failed to connect publisher to broker");

    // verify that a message sent by the publisher is received by the subscriber
    publisher
        .send(message_to_send.clone())
        .await
        .expect("failed to publish message");

    assert!(
        tokio::time::timeout(
            Duration::from_millis(1000),
            message_received_barrier.notified()
        )
        .await
        .is_ok(),
        "did not receive message before timeout"
    );

    // now restart the broker to simulate a (temporary) disconnect
    mosquitto.stop().await.expect("Failed to stop Mosquitto");
    debug!("stopped Mosquitto, waiting for 1 second before restarting");
    tokio::time::sleep(Duration::from_secs(1)).await;
    debug!("restarting Mosquitto");
    mosquitto
        .start()
        .await
        .expect("Failed to start Mosquitto again");
    debug!("Mosquitto restarted");

    // wait for the connections and subscriptions to be reestablished
    tokio::time::timeout(
        Duration::from_secs(10),
        futures::future::join(
            common::connection_established(publisher.clone()),
            common::connection_established(subscriber.clone()),
        ),
    )
    .await
    .expect("transports did not reconnect in time");

    // publish the message again
    publisher
        .send(message_to_send)
        .await
        .expect("failed to publish message after reconnect");

    // [utest->req~up-transport-mqtt5-reconnection~1]
    // wait for the message to be received again
    assert!(
        tokio::time::timeout(
            Duration::from_millis(1000),
            message_received_barrier.notified()
        )
        .await
        .is_ok(),
        "did not receive second message before timeout"
    );
}
