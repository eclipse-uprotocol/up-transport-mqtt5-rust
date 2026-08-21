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

use serial_test::serial;
use tracing::debug;
use up_rust::{MockUListener, UCode, UMessageBuilder, UTransport, UUri};

mod common;

// User credentials to be used with the Mosquitto broker
// created using https://dmelo.eu/blog/mosquitto_passwd_gen/
// username: publisher, password: uprotocol
// username: subscriber, password: uprotocol
const PASSWD_FILE: &str = r#"
    publisher:$7$101$njCnRQhFsQbDh5NF$pWuZpNcTNwM1tbDePb+UTww3Y6cL5jfC0tXVAUvRTc5wTDjE9PnzTQmyxW/1RA86rHvtXywrqlJS2cWyELeI1Q==
    subscriber:$7$101$zACO8Nh9VM+YjQJR$hZZSQ+Qlp1Gvr4Fau00dQTyiNmBAkHd/uE2Ucd1uZ1ST870Y5WH5cQ33/VnbfJgURePpMBSBkTdaQyqMCIe5GQ==
  "#;

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
    let _ = tracing_subscriber::fmt::try_init();

    // fixture
    let mosquitto =
        common::start_mosquitto(mosquitto_config, Some(PASSWD_FILE), None, Some(15000)).await;
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
        Some("subscriber"),
        Some("uprotocol"),
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
        Some("publisher"),
        Some("uprotocol"),
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

#[tokio::test]
#[cfg_attr(not(docker_available), ignore)]
// This test requires Docker to run the Mosquitto MQTT broker.
async fn test_connect_fails_for_wrong_credentials() {
    let _ = tracing_subscriber::fmt::try_init();

    // fixture
    let mosquitto = common::start_mosquitto(None, Some(PASSWD_FILE), None, None).await;
    let port = mosquitto.port();

    let publisher = common::create_up_transport_mqtt(
        "publisher",
        port,
        None,
        Some("publisher"),
        Some("wrong_password"),
    )
    .await
    .expect("failed to create MQTT5 transport");

    assert!(
        publisher.connect().await.is_err_and(|err| {
            debug!("connect attempt failed: {err:?}");
            // this is not generally required by the MQTT 5 spec but we know that
            // the Mosquitto broker returns a CONNACK with reason code 135 instead
            // of simply closing the connection
            err.get_code() == UCode::PERMISSION_DENIED
        }),
        "expected connection to fail due to wrong credentials"
    );

    publisher.shutdown().await;
}

#[tokio::test]
#[cfg_attr(not(docker_available), ignore)]
// This test requires Docker to run the Mosquitto MQTT broker.
async fn test_publish_fails_if_unauthorized() {
    let _ = tracing_subscriber::fmt::try_init();

    // fixture
    let acl_file = r#"
        user publisher
        topic write publisher/8000/A/#
    "#;
    let mosquitto = common::start_mosquitto(None, Some(PASSWD_FILE), Some(acl_file), None).await;

    let publisher = common::create_up_transport_mqtt(
        "publisher",
        mosquitto.port(),
        None,
        Some("publisher"),
        Some("uprotocol"),
    )
    .await
    .expect("failed to create transport at sending end");

    publisher
        .connect()
        .await
        .expect("failed to connect publisher to broker");

    let authorized_source = UUri::from_str("/A8000/2/8A50").unwrap();
    assert!(
        publisher
            .send(UMessageBuilder::publish(authorized_source).build().unwrap())
            .await
            .is_ok(),
        "expected publishing to topic to succeed with correct authority"
    );

    let unauthorized_source = UUri::from_str("/100/1/B500").unwrap();
    assert!(
        publisher
            .send(
                UMessageBuilder::publish(unauthorized_source)
                    .build()
                    .unwrap()
            )
            .await
            .is_err_and(|err| {
                debug!("failed to publish message: {err:?}");
                err.get_code() == UCode::PERMISSION_DENIED
            }),
        "expected publishing to topic to fail due to missing authority"
    );

    publisher.shutdown().await;
}
