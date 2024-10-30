use std::{str::FromStr, sync::{Arc, Mutex}, time::Duration};

use tokio::time::sleep;
use up_rust::{UMessageBuilder, UPayloadFormat, UTransport, UUri};

mod test_lib;

#[tokio::test(flavor = "multi_thread")]
async fn test_publish_and_subscribe() {

    let target_data = "TEST";

    let publisher = test_lib::create_up_transport_mqtt("Publisher").await.unwrap();
    let subscriber = test_lib::create_up_transport_mqtt("Subscriber").await.unwrap();

    let source = UUri::from_str("//Publisher/A8000/2/8A50").expect("Failed to create source filter");
    let source_filter = UUri::from_str("//Publisher/A8000/2/8A50").expect("Failed to create source filter");

    let listener = Arc::new(
        test_lib::TestListener { recv_data: Arc::new(Mutex::new(String::new())) }
    );

    subscriber.register_listener(&source_filter, None, listener.clone()).await.unwrap();

    sleep(Duration::from_millis(1000)).await;

    let umessage = UMessageBuilder::publish(source)
        .build_with_payload(target_data, UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
        .unwrap();
    publisher.send(umessage).await.unwrap();

    sleep(Duration::from_millis(1000)).await;

    assert_eq!(listener.get_recv_data(), target_data)

}