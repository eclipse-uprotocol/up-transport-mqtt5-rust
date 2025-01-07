use std::{str::FromStr, sync::Arc, time::Duration};

use tokio::sync::Notify;
use up_rust::{MockUListener, UMessageBuilder, UTransport, UUri};

mod common;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "should only be executed with an MQTT broker running"]
async fn test_publish_and_subscribe() {
    let message_received = Arc::new(Notify::new());
    let message_received_clone = message_received.clone();
    let mut listener = MockUListener::new();
    listener
        .expect_on_receive()
        .once()
        .return_once(move |_msg| {
            message_received_clone.notify_one();
        });

    let subscriber = common::create_up_transport_mqtt("Subscriber")
        .await
        .expect("failed to create transport at receiving end");
    let source_filter =
        UUri::from_str("//Publisher/A8000/2/FFFF").expect("Failed to create source filter");
    subscriber
        .register_listener(&source_filter, None, Arc::new(listener))
        .await
        .unwrap();

    let publisher = common::create_up_transport_mqtt("Publisher")
        .await
        .expect("failed to create transport at sending end");
    let source = UUri::from_str("//Publisher/A8000/2/8A50").unwrap();
    let umessage = UMessageBuilder::publish(source).build().unwrap();
    publisher
        .send(umessage)
        .await
        .expect("failed to publish message");

    tokio::time::timeout(Duration::from_millis(1000), message_received.notified())
        .await
        .expect("did not receive published message before timeout");
}
