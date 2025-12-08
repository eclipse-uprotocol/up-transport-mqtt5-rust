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

use std::str::FromStr;

use protobuf::{Enum, EnumOrUnknown, MessageField};
use up_rust::{
    UAttributes, UAttributesValidators, UCode, UMessageType, UPayloadFormat, UPriority, UStatus,
    UUri, UUID,
};

const CURRENT_UPROTOCOL_MAJOR_VERSION: u8 = 1;

/// Constants defining the protobuf field numbers for `UAttributes`.
const KEY_UPROTOCOL_VERSION: &str = "uP";
const KEY_MESSAGE_ID: &str = "1";
const KEY_TYPE: &str = "2";
const KEY_SOURCE: &str = "3";
const KEY_SINK: &str = "4";
const KEY_PRIORITY: &str = "5";
const KEY_TTL: &str = "6";
const KEY_PERMISSION_LEVEL: &str = "7";
const KEY_COMMSTATUS: &str = "8";
const KEY_TOKEN: &str = "10";
const KEY_TRACEPARENT: &str = "11";

/// Adds a user property to MQTT properties.
fn add_user_property(
    properties: &mut paho_mqtt::Properties,
    key: &str,
    value: &str,
    error_message: &str,
) -> Result<(), UStatus> {
    properties
        .push_string_pair(paho_mqtt::PropertyCode::UserProperty, key, value)
        .map_err(|e| UStatus::fail_with_code(UCode::INTERNAL, format!("{error_message}: {e:?}")))
}

#[cfg_attr(test, mockall::automock)]
pub(crate) trait MessageMapper: Send + Sync {
    /// Creates MQTT 5 header properties from uProtocol message meta data
    /// as defined by uProtocol's MQTT5 transport specification.
    ///
    /// # Arguments
    /// * `attributes` - The message meta data.
    ///
    /// # Errors
    ///
    /// Returns an error if the given meta data are invalid or cannot
    /// be mapped to MQTT properties.
    fn create_mqtt_properties_from_uattributes(
        &self,
        attributes: &UAttributes,
    ) -> Result<paho_mqtt::Properties, UStatus>;

    /// Creates uProtocol message meta data from MQTT header properties.
    ///
    /// # Arguments
    /// * `props` - MQTT properties to get meta data from.
    ///
    /// # Errors
    ///
    /// Returns an error if the MQTT header properties cannot be mapped to valid uProtocol message meta data.
    fn create_uattributes_from_mqtt_properties(
        &self,
        props: &paho_mqtt::Properties,
    ) -> Result<UAttributes, UStatus>;
}

#[derive(Default)]
pub(crate) struct DefaultMessageMapper;

impl MessageMapper for DefaultMessageMapper {
    // [impl->dsn~up-transport-mqtt5-attributes-mapping~1]
    fn create_mqtt_properties_from_uattributes(
        &self,
        attributes: &UAttributes,
    ) -> Result<paho_mqtt::Properties, UStatus> {
        // No need to start conversion if attributes are invalid
        // [impl->dsn~utransport-send-error-invalid-parameter~1]
        UAttributesValidators::get_validator_for_attributes(attributes)
            .validate(attributes)
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    format!("Invalid uAttributes, err: {e:?}"),
                )
            })?;

        let mut properties = paho_mqtt::Properties::new();

        // Add uProtocol major version
        add_user_property(
            &mut properties,
            KEY_UPROTOCOL_VERSION,
            CURRENT_UPROTOCOL_MAJOR_VERSION.to_string().as_str(),
            "Failed to add uProtocol major version to MQTT User Properties",
        )?;

        // Add TTL
        if let Some(ttl) = attributes.ttl {
            properties
                .push_u32(
                    paho_mqtt::PropertyCode::MessageExpiryInterval,
                    ttl.div_ceil(1000),
                )
                .map_err(|e| {
                    UStatus::fail_with_code(
                        UCode::INTERNAL,
                        format!("Failed to create Message Expiry Interval property: {e:?}"),
                    )
                })?;

            if ttl % 1000 > 0 {
                // TTL does not have full second granularity so we need to also add
                // a dedicated user property to be able to recreate the original
                // value at the receiving end
                add_user_property(
                    &mut properties,
                    KEY_TTL,
                    ttl.to_string().as_str(),
                    "Failed to add TTL to MQTT User Properties",
                )?;
            }
        }

        add_user_property(
            &mut properties,
            KEY_MESSAGE_ID,
            &attributes.id.to_hyphenated_string(),
            "Failed to add message ID to mqtt User Properties",
        )?;

        if let Ok(message_type) = attributes.type_.enum_value() {
            add_user_property(
                &mut properties,
                KEY_TYPE,
                &message_type.to_cloudevent_type(),
                "Failed to add message type to MQTT User Properties",
            )?;
        }

        add_user_property(
            &mut properties,
            KEY_SOURCE,
            &attributes.source.to_uri(false),
            "Failed to add message source to MQTT User Properties",
        )?;

        if let Some(sink) = attributes.sink.as_ref() {
            add_user_property(
                &mut properties,
                KEY_SINK,
                &sink.to_uri(false),
                "Failed to add message sink to MQTT User Properties",
            )?;
        }

        let prio = attributes.priority.enum_value().map_err(|v| {
            UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                format!("message has unsupported priority code [{v}]"),
            )
        })?;
        if prio != UPriority::UPRIORITY_UNSPECIFIED {
            add_user_property(
                &mut properties,
                KEY_PRIORITY,
                &prio.to_priority_code(),
                "Failed to add message priority to MQTT User Properties",
            )?;
        }

        if let Some(permission_level) = &attributes.permission_level {
            add_user_property(
                &mut properties,
                KEY_PERMISSION_LEVEL,
                &permission_level.to_string(),
                "Failed to add message permission level to MQTT User Properties",
            )?;
        }

        if let Some(comm_status) = &attributes.commstatus {
            add_user_property(
                &mut properties,
                KEY_COMMSTATUS,
                &comm_status.value().to_string(),
                "Failed to add message comm status to MQTT User Properties",
            )?;
        }

        if let Some(req_id) = attributes.reqid.as_ref() {
            properties
                .push_binary::<Vec<u8>>(paho_mqtt::PropertyCode::CorrelationData, req_id.into())
                .map_err(|e| {
                    UStatus::fail_with_code(
                        UCode::INTERNAL,
                        format!("Failed to create Correlation Data property: {e:?}"),
                    )
                })?;
        }

        if let Some(token) = &attributes.token {
            add_user_property(
                &mut properties,
                KEY_TOKEN,
                token,
                "Failed to add message token to MQTT User Properties",
            )?;
        }

        if let Some(traceparent) = &attributes.traceparent {
            add_user_property(
                &mut properties,
                KEY_TRACEPARENT,
                traceparent,
                "Failed to add message traceparent to MQTT User Properties",
            )?;
        }

        if let Ok(format) = attributes.payload_format.enum_value() {
            if format != UPayloadFormat::UPAYLOAD_FORMAT_UNSPECIFIED {
                properties
                    .push_string(
                        paho_mqtt::PropertyCode::ContentType,
                        &format.value().to_string(),
                    )
                    .map_err(|e| {
                        UStatus::fail_with_code(
                            UCode::INTERNAL,
                            format!("Failed to create Content Type property: {e:?}"),
                        )
                    })?;
            }
        }

        Ok(properties)
    }

    // [impl->dsn~up-transport-mqtt5-attributes-mapping~1]
    fn create_uattributes_from_mqtt_properties(
        &self,
        props: &paho_mqtt::Properties,
    ) -> Result<UAttributes, UStatus> {
        let uprotocol_major_version = props.find_user_property(KEY_UPROTOCOL_VERSION)
        .ok_or_else(||UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "MQTT message does not contain uProtocol version identifier"))
        .and_then(|s| s.parse::<u8>().map_err(|err| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, format!("Failed to map UserProperty {KEY_UPROTOCOL_VERSION} to uProtocol major version: {err}"))))?;
        if uprotocol_major_version != CURRENT_UPROTOCOL_MAJOR_VERSION {
            return Err(UStatus::fail_with_code(UCode::INVALID_ARGUMENT, format!("MQTT message contains unsupported uProtocol major version [expected: {CURRENT_UPROTOCOL_MAJOR_VERSION}, found: {uprotocol_major_version}")));
        }

        let mut attributes = UAttributes {
            token: props.find_user_property(KEY_TOKEN),
            traceparent: props.find_user_property(KEY_TRACEPARENT),
            ..Default::default()
        };

        if let Some(message_id) = props.find_user_property(KEY_MESSAGE_ID) {
            let id = UUID::from_str(&message_id).map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    format!("Failed to map UserProperty {KEY_MESSAGE_ID} to Message ID: {e}"),
                )
            })?;
            attributes.id = MessageField::from(Some(id));
        }

        if let Some(message_type_str) = props.find_user_property(KEY_TYPE) {
            attributes.type_ = UMessageType::try_from_cloudevent_type(message_type_str)
                .map_err(|e| {
                    UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        format!("Failed to map UserProperty {KEY_TYPE} to Message Type: {e}"),
                    )
                })
                .map(EnumOrUnknown::from)?;
        }

        if let Some(source_string) = props.find_user_property(KEY_SOURCE) {
            attributes.source = UUri::from_str(&source_string)
                .map_err(|err| {
                    UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        format!("fAILED to map UserProperty {KEY_SOURCE} to Message Source: {err}"),
                    )
                })
                .map(MessageField::some)?;
        }

        if let Some(sink_string) = props.find_user_property(KEY_SINK) {
            attributes.sink = UUri::from_str(&sink_string)
                .map_err(|err| {
                    UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        format!("Failed to map UserProperty {KEY_SINK} to Message Sink: {err}"),
                    )
                })
                .map(MessageField::some)?;
        }

        if let Some(priority_string) = props.find_user_property(KEY_PRIORITY) {
            attributes.priority =
                UPriority::try_from_priority_code(priority_string)
                    .map_err(|e| {
                        UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    format!("Failed to map UserProperty {KEY_PRIORITY} to Message Priority: {e}"),
                )
                    })
                    .map(EnumOrUnknown::from)?;
        } else {
            // [impl->dsn~up-attributes-priority~1]
            // it is sufficient to set to UNSPECIFIED because according to the spec,
            // a message without a (concrete) priority, belongs to class CS1 by default
            attributes.priority = EnumOrUnknown::from(UPriority::UPRIORITY_UNSPECIFIED);
        }

        if let Some(ttl_string) = props.find_user_property(KEY_TTL) {
            // Add the TTL UAttribute from TTL user property if it is set
            attributes.ttl = Some(ttl_string.parse::<u32>().map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    format!("Failed to map UserProperty {KEY_TTL} to Message TTL: {e}"),
                )
            })?);
        } else if let Some(message_expiry_interval) = props
            .get(paho_mqtt::PropertyCode::MessageExpiryInterval)
            .and_then(|prop| prop.get_u32())
        {
            // otherwise, fall back to the MessageExpiryInterval if available
            attributes.ttl = message_expiry_interval.checked_mul(1000).or(Some(u32::MAX));
        }

        if let Some(permission_string) = props.find_user_property(KEY_PERMISSION_LEVEL) {
            attributes.permission_level = permission_string
            .parse()
            .map_err(|err| {
                UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    format!(
                        "Failed to map UserProperty {KEY_PERMISSION_LEVEL} to Permission Level: {err}"
                    ),
                )
            })
            .map(Option::Some)?;
        }

        if let Some(comm_status_string) = props.find_user_property(KEY_COMMSTATUS) {
            attributes.commstatus = comm_status_string
            .parse::<i32>()
            .map_err(|err| {
                UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    format!("Failed to map UserProperty {KEY_COMMSTATUS} to CommStatus: {err}"),
                )
            })
            .and_then(|v| {
                UCode::from_i32(v).ok_or_else(||{
                    UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        format!("Failed to map UserProperty {KEY_COMMSTATUS} to CommStatus: not a valid UCode [{v}]"),
                    )
                })
            })
            .map(EnumOrUnknown::from)
            .map(Option::Some)?;
        }

        if let Some(req_id) = props.get_binary(paho_mqtt::PropertyCode::CorrelationData) {
            let uuid = UUID::try_from(req_id)
                .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;
            attributes.reqid = Some(uuid).into();
        }

        if let Some(payload_format_string) = props.get_string(paho_mqtt::PropertyCode::ContentType)
        {
            attributes.payload_format = payload_format_string
            .parse::<i32>()
            .map_err(|err| UStatus::fail_with_code(
              UCode::INVALID_ARGUMENT,
              format!("Failed to map Content Type to Message Payload Format: {err}")))
            .and_then(|v| {
                UPayloadFormat::from_i32(v).ok_or_else(||UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    format!("Failed to map Content Type to Message Payload Format: not a valid payload format code [{v}]"),
                ))
            })
            .map(EnumOrUnknown::from)?;
        }

        // Validate the reconstructed attributes
        let validator = UAttributesValidators::get_validator_for_attributes(&attributes);
        validator.validate(&attributes).map_err(|e| {
            UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                format!("Failed to map message attributes: {e:?}"),
            )
        })?;

        // [impl->dsn~up-attributes-ttl~1]
        // [impl->dsn~up-attributes-ttl-timeout~1]
        attributes.check_expired().map_err(|_err| {
            UStatus::fail_with_code(UCode::DEADLINE_EXCEEDED, "message has expired")
        })?;

        Ok(attributes)
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    const MSG_TOKEN: &str = "the token";
    const MSG_TRACEPARENT: &str = "traceparent";
    const MSG_PERMISSION_LEVEL: u32 = 15;

    // Helper function used to create a UAttributes object and corresponding
    // MQTT properties object for testing and comparison
    #[allow(clippy::too_many_arguments)]
    fn create_test_uattributes_and_properties(
        major_version: Option<u8>,
        type_: Option<UMessageType>,
        id: Option<&UUID>,
        source: Option<&str>,
        sink: Option<&str>,
        priority: Option<UPriority>,
        ttl: Option<u32>, // milliseconds
        perm_level: Option<u32>,
        commstatus: Option<UCode>,
        reqid: Option<&UUID>,
        token: Option<&str>,
        traceparent: Option<&str>,
        payload_format: Option<UPayloadFormat>,
    ) -> (UAttributes, paho_mqtt::Properties) {
        let uattributes = create_uattributes(
            type_,
            id,
            source,
            sink,
            priority,
            ttl,
            perm_level,
            commstatus,
            reqid,
            token,
            traceparent,
            payload_format,
        );

        let properties = create_mqtt_properties(
            major_version,
            type_,
            id,
            source,
            sink,
            priority,
            ttl,
            perm_level,
            commstatus,
            reqid,
            token,
            traceparent,
            payload_format,
        );

        (uattributes, properties)
    }

    // Helper function to construct UAttributes object for testing.
    #[allow(clippy::too_many_arguments)]
    fn create_uattributes(
        type_: Option<UMessageType>,
        id: Option<&UUID>,
        source: Option<&str>,
        sink: Option<&str>,
        priority: Option<UPriority>,
        ttl: Option<u32>, // milliseconds
        permission_level: Option<u32>,
        commstatus: Option<UCode>,
        reqid: Option<&UUID>,
        token: Option<&str>,
        traceparent: Option<&str>,
        payload_format: Option<UPayloadFormat>,
    ) -> UAttributes {
        UAttributes {
            commstatus: commstatus.map(EnumOrUnknown::from),
            id: id.map(|id| id.to_owned()).into(),
            payload_format: EnumOrUnknown::from(payload_format.unwrap_or_default()),
            priority: EnumOrUnknown::from(priority.unwrap_or(UPriority::UPRIORITY_UNSPECIFIED)),
            permission_level,
            reqid: reqid.map(|uuid| uuid.to_owned()).into(),
            source: source
                .map(|uri| UUri::from_str(uri).expect("expected valid source URI"))
                .into(),
            sink: sink
                .map(|uri| UUri::from_str(uri).expect("expected valid sink URI"))
                .into(),
            token: token.map(|s| s.to_owned()),
            traceparent: traceparent.map(|s| s.to_owned()),
            ttl,
            type_: EnumOrUnknown::from(type_.unwrap_or_default()),
            ..Default::default()
        }
    }

    // Helper function to create mqtt properties for testing.
    #[allow(clippy::too_many_arguments)]
    fn create_mqtt_properties(
        major_version: Option<u8>,
        type_: Option<UMessageType>,
        id: Option<&UUID>,
        source: Option<&str>,
        sink: Option<&str>,
        priority: Option<UPriority>,
        ttl: Option<u32>, // milliseconds
        perm_level: Option<u32>,
        commstatus: Option<UCode>,
        reqid: Option<&UUID>,
        token: Option<&str>,
        traceparent: Option<&str>,
        payload_format: Option<UPayloadFormat>,
    ) -> paho_mqtt::Properties {
        let mut properties = paho_mqtt::Properties::new();

        if let Some(version) = major_version {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_UPROTOCOL_VERSION,
                    version.to_string().as_str(),
                )
                .unwrap();
        }

        if let Some(type_val) = type_ {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_TYPE,
                    &type_val.to_cloudevent_type(),
                )
                .unwrap();
        }

        if let Some(id_val) = id {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_MESSAGE_ID,
                    &id_val.to_hyphenated_string(),
                )
                .unwrap();
        }

        if let Some(source_val) = source {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_SOURCE,
                    source_val,
                )
                .unwrap();
        }
        if let Some(sink_val) = sink {
            properties
                .push_string_pair(paho_mqtt::PropertyCode::UserProperty, KEY_SINK, sink_val)
                .unwrap();
        }
        if let Some(priority_val) = priority.filter(|v| *v != UPriority::UPRIORITY_UNSPECIFIED) {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_PRIORITY,
                    &priority_val.to_priority_code(),
                )
                .unwrap();
        }
        if let Some(v) = ttl {
            properties
                .push_u32(
                    paho_mqtt::PropertyCode::MessageExpiryInterval,
                    v.div_ceil(1000),
                )
                .unwrap();
            if v % 1000 > 0 {
                properties
                    .push_string_pair(
                        paho_mqtt::PropertyCode::UserProperty,
                        KEY_TTL,
                        &v.to_string(),
                    )
                    .unwrap();
            }
        }
        if let Some(perm_level_val) = perm_level {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_PERMISSION_LEVEL,
                    &perm_level_val.to_string(),
                )
                .unwrap();
        }
        if let Some(commstatus_val) = commstatus {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_COMMSTATUS,
                    &commstatus_val.value().to_string(),
                )
                .unwrap();
        }
        if let Some(reqid_val) = reqid {
            properties
                .push_binary::<Vec<u8>>(paho_mqtt::PropertyCode::CorrelationData, reqid_val.into())
                .inspect_err(|err| println!("{err}"))
                .expect("Failed to set Correlation Data property");
        }
        if let Some(token_val) = token {
            properties
                .push_string_pair(paho_mqtt::PropertyCode::UserProperty, KEY_TOKEN, token_val)
                .unwrap();
        }
        if let Some(traceparent_val) = traceparent {
            properties
                .push_string_pair(
                    paho_mqtt::PropertyCode::UserProperty,
                    KEY_TRACEPARENT,
                    traceparent_val,
                )
                .unwrap();
        }
        if let Some(payload_format_val) = payload_format {
            properties
                .push_string(
                    paho_mqtt::PropertyCode::ContentType,
                    &payload_format_val.value().to_string(),
                )
                .unwrap();
        }

        properties
    }

    /// Verifies that two sets of MQTT properties contain the same items.
    fn assert_mqtt_properties(
        properties: &paho_mqtt::Properties,
        expected_properties: &paho_mqtt::Properties,
    ) {
        assert_eq!(
            properties
                .get(paho_mqtt::PropertyCode::MessageExpiryInterval)
                .map(|prop| prop.get_u32()),
            expected_properties
                .get(paho_mqtt::PropertyCode::MessageExpiryInterval)
                .map(|prop| prop.get_u32())
        );
        properties.user_iter().for_each(|(key, value)| {
            assert_eq!(expected_properties.find_user_property(&key), Some(value));
        });
    }

    //
    // MQTT Properties -> UAttributes
    //

    #[test_case(
        create_test_uattributes_and_properties(
            Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some(&UUID::build()),
            Some("//vin.vehicles/A8000/2/8A50"),
            None,
            Some(UPriority::UPRIORITY_CS5),
            None, None, None, None, None, None,
            Some(UPayloadFormat::UPAYLOAD_FORMAT_TEXT),
        ),
        None;
        "for valid Publish message"
    )]
    // [utest->dsn~up-attributes-priority~1]
    // [utest->dsn~up-attributes-ttl~1]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
            Some(UMessageType::UMESSAGE_TYPE_NOTIFICATION),
            Some(&UUID {
                // timestamp: 1000ms since UNIX epoch
                msb: 0x0000000010007000_u64,
                lsb: 0x8010101010101a1a_u64,
                ..Default::default()
            }),
            Some("//vin.vehicles/A8000/2/1A50"),
            Some("//vin.vehicles/B8000/3/0"),
            None,
            // do not expire
            Some(0),
            None, None, None, None,
            Some(MSG_TRACEPARENT),
            None
        ),
        None;
        "for valid Notification"
    )]
    // [utest->dsn~up-attributes-ttl-timeout~1]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
            Some(UMessageType::UMESSAGE_TYPE_REQUEST),
            Some(&UUID::build()),
            Some("//vin.vehicles/A8000/2/0"),
            Some("//vin.vehicles/B8000/3/1B50"),
            Some(UPriority::UPRIORITY_CS4),
            Some(5400),
            Some(MSG_PERMISSION_LEVEL),
            None, None,
            Some(MSG_TOKEN),
            Some(MSG_TRACEPARENT),
            Some(UPayloadFormat::UPAYLOAD_FORMAT_RAW)
        ),
        None;
        "for valid Request"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
            Some(UMessageType::UMESSAGE_TYPE_RESPONSE),
            Some(&UUID::build()),
            Some("//vin.vehicles/B8000/3/1B50"),
            Some("//vin.vehicles/A8000/2/0"),
            Some(UPriority::UPRIORITY_CS4),
            Some(3000),
            None,
            Some(UCode::UNIMPLEMENTED),
            Some(&UUID::build()),
            None,
            Some(MSG_TRACEPARENT),
            None
        ),
        None;
        "for valid Response"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some(&UUID::build()),
            // source must have resource ID >= 0x8000
            Some("//vin.vehicles/A8000/2/1A50"),
            None, None, None, None, None, None, None, None, None
        ),
        Some(UCode::INVALID_ARGUMENT);
        "fails for Publish message with invalid source URI"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(CURRENT_UPROTOCOL_MAJOR_VERSION + 1),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some(&UUID::build()),
            Some("//vin.vehicles/A8000/2/AA50"),
            None, None, None, None, None, None, None, None, None
        ),
        Some(UCode::INVALID_ARGUMENT);
        "fails for Publish message with invalid uProtocol version"
    )]
    // [utest->dsn~up-attributes-ttl-timeout~1]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some(&UUID {
                // timestamp: 1000ms since UNIX epoch
                msb: 0x0000000010007000_u64,
                lsb: 0x8010101010101a1a_u64,
                ..Default::default()
            }),
            Some("//vin.vehicles/A8000/2/AA50"),
            None,
            None,
            // message expiry interval: 12.5s
            // i.e. this message has expired decades ago
            Some(12500),
            None, None, None, None, None, None
        ),
        Some(UCode::DEADLINE_EXCEEDED);
        "fails for expired Publish message"
    )]
    // [utest->dsn~up-transport-mqtt5-attributes-mapping~1]
    fn test_create_uattributes_from_mqtt_properties(
        (expected_attributes, mqtt_properties): (UAttributes, paho_mqtt::Properties),
        expected_error_code: Option<UCode>,
    ) {
        let mapper = DefaultMessageMapper;
        let attributes_result = mapper.create_uattributes_from_mqtt_properties(&mqtt_properties);
        if let Some(code) = expected_error_code {
            assert!(attributes_result.is_err_and(|err| err.get_code() == code))
        } else {
            assert!(attributes_result.is_ok_and(|attribs| attribs == expected_attributes));
        }
    }

    //
    // UAttributes -> MQTT Properties
    //

    #[test]
    // [utest->dsn~up-transport-mqtt5-attributes-mapping~1]
    fn test_create_properties_from_invalid_attributes_fails() {
        let invalid_attributes = UAttributes {
            type_: EnumOrUnknown::from(UMessageType::UMESSAGE_TYPE_PUBLISH),
            // Publish message must have source field
            source: MessageField::none(),
            ..Default::default()
        };
        let mapper = DefaultMessageMapper;
        assert!(mapper
            .create_mqtt_properties_from_uattributes(&invalid_attributes)
            .is_err_and(|err| err.get_code() == UCode::INVALID_ARGUMENT));
    }

    #[test_case(create_test_uattributes_and_properties(
        Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
        Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
        Some(&UUID::build()),
        Some("/A10D/4/B3AA"),
        None,
        Some(UPriority::UPRIORITY_CS5),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
    );"for Publish message")]
    #[test_case(create_test_uattributes_and_properties(
        Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
        Some(UMessageType::UMESSAGE_TYPE_NOTIFICATION),
        Some(&UUID::build()),
        Some("/A10D/4/B3AA"),
        Some("/103/2/0"),
        None,
        None,
        None,
        None,
        None,
        Some(MSG_TRACEPARENT),
        None,
        None
    );"for Notification message")]
    #[test_case(create_test_uattributes_and_properties(
        Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
        Some(UMessageType::UMESSAGE_TYPE_REQUEST),
        Some(&UUID::build()),
        Some("/A10D/4/0"),
        Some("/103/2/71A3"),
        Some(UPriority::UPRIORITY_CS4),
        Some(2000),
        Some(MSG_PERMISSION_LEVEL),
        None,
        None,
        Some(MSG_TOKEN),
        Some(MSG_TRACEPARENT),
        Some(UPayloadFormat::UPAYLOAD_FORMAT_RAW)
    );"for RPC Request message")]
    #[test_case(create_test_uattributes_and_properties(
        Some(CURRENT_UPROTOCOL_MAJOR_VERSION),
        Some(UMessageType::UMESSAGE_TYPE_RESPONSE),
        Some(&UUID::build()),
        Some("/103/2/71A3"),
        Some("/A10D/4/0"),
        Some(UPriority::UPRIORITY_CS4),
        Some(4150),
        None,
        Some(UCode::UNIMPLEMENTED),
        Some(&UUID::build()),
        None,
        Some(MSG_TRACEPARENT),
        None
    );"for RPC Response message")]
    // [utest->dsn~up-transport-mqtt5-attributes-mapping~1]
    fn test_create_mqtt_properties_from_uattributes(
        (attributes, expected_mqtt_properties): (UAttributes, paho_mqtt::Properties),
    ) {
        let mapper = DefaultMessageMapper;
        let mqtt_properties = mapper
            .create_mqtt_properties_from_uattributes(&attributes)
            .expect("failed to create MQTT properties from message attributes");
        assert_mqtt_properties(&mqtt_properties, &expected_mqtt_properties);
    }
}
