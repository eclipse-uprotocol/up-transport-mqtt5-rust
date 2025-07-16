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
    collections::{HashMap, HashSet},
    sync::Arc,
};

use log::debug;
use slab::Slab;
use up_rust::{ComparableListener, UCode, UStatus};

pub(crate) type SubscriptionIdentifier = u16;
type SubscriptionTopics = Slab<(String, HashSet<ComparableListener>)>;
type TopicListeners = paho_mqtt::TopicMatcher<HashSet<ComparableListener>>;

const NO_VALID_U16: &str = "SubscriptionId not a valid u16";

pub(crate) struct RegisteredListeners {
    /// Mapping of subscription identifiers to topic filters.
    subscription_topics: SubscriptionTopics,
    /// Mapping of topic filters to listeners.
    topic_listeners: TopicListeners,
    // [impl->req~utransport-registerlistener-max-listeners~1]
    max_listeners_per_subscription: usize,
}

impl Default for RegisteredListeners {
    fn default() -> Self {
        Self::new(10, 5)
    }
}

impl RegisteredListeners {
    pub(crate) fn new(max_subscriptions: u16, max_listeners_per_subscription: u16) -> Self {
        // Since subscribtion ids start at 1 we can only create a maximum number of u16:MAX - 1
        assert!(max_subscriptions < u16::MAX);
        // Note that this assert implies that the slab will at most contain u16:MAX elements hence any
        // index coming from iteration is a valid u16.
        let mut sub_topics = SubscriptionTopics::with_capacity((max_subscriptions + 1).into());

        // MQTT subscription identifiers start with 1 so we add a dummy value
        sub_topics.insert((String::new(), HashSet::new()));

        Self {
            subscription_topics: sub_topics,
            topic_listeners: TopicListeners::new(),
            max_listeners_per_subscription: max_listeners_per_subscription.into(),
        }
    }

    /// Resets this registry to its initial state.
    /// This includes removing all registered listeners, topic filters and subscription identifiers.
    pub(crate) fn clear(&mut self) {
        self.topic_listeners.clear();
        self.subscription_topics.clear();
    }

    fn find_subscription_id(&self, topic_filter: &str) -> Option<SubscriptionIdentifier> {
        self.subscription_topics.iter().find_map(|(idx, v)| {
            if v.0 == topic_filter {
                // cannot fail because the slab has been initialized
                // with a capacity <= u16::MAX
                Some(u16::try_from(idx).expect(NO_VALID_U16))
            } else {
                None
            }
        })
    }

    /// Returns a subscription ID back to the pool of free/unused subscription IDs.
    ///
    /// Also removes all listeners registered for the given topic filter as well as
    /// the mapping of the subscription identifier to the topic filter.
    ///
    /// This function should be invoked after a failed attempt to subscribe to the
    /// topic filter with the subscription identifier.
    ///
    /// # Arguments
    /// * `id` - The subscription ID to release.
    /// * `topic_filter` - The topic filter to remove all listeners for.
    pub(crate) fn release_subscription_id(
        &mut self,
        id: SubscriptionIdentifier,
        topic_filter: &str,
    ) {
        self.topic_listeners.remove(topic_filter);
        self.subscription_topics.remove(id.into());
    }

    /// Adds a listener for a given topic filter.
    ///
    /// The same listener instance can be registered using multiple topic filters.
    /// The listener will be invoked once per message for each distinct topic filter that
    /// the listener has been registered for and which matches the topic that the message
    /// has been published to.
    ///
    /// # Returns
    ///
    /// A newly assigned subscription identifier to be used for subscribing to the topic
    /// filter with the MQTT broker, or `None` if the listener has been added to an existing
    /// subscription for the topic filter.
    ///
    /// Note that the [`Self::release_subscription_id`] function must be invoked, if subscribing
    /// to the topic filter with the MQTT broker fails. Otherwise, the pool of available
    /// subscription IDs might exhaust early.
    ///
    /// # Errors
    ///
    /// Returns an error with `UCode::RESOURCE_EXHAUSTED` if the maximum number of topic filters
    /// or the maximum number of listeners per topic filter has already been reached.
    pub(crate) fn add_listener(
        &mut self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> Result<Option<SubscriptionIdentifier>, UStatus> {
        let comp_listener = ComparableListener::new(listener);

        // [impl->dsn~utransport-registerlistener-idempotent~1]
        // [impl->dsn~utransport-registerlistener-listener-reuse~1]
        // [impl->dsn~utransport-registerlistener-number-of-listeners~1]

        // If we know the subscription id already we insert the listener there
        if let Some(sub_id) = self.find_subscription_id(topic_filter) {
            // OK to unwrap since we checked that we know the sub_id
            self.subscription_topics
                .get_mut(sub_id.into())
                .unwrap()
                .1
                .insert(comp_listener.clone());
        }
        if let Some(listeners) = self.topic_listeners.get_mut(topic_filter) {
            // [impl->dsn~utransport-registerlistener-error-resource-exhausted~1]
            if listeners.len() >= self.max_listeners_per_subscription {
                return Err(UStatus::fail_with_code(
                    UCode::RESOURCE_EXHAUSTED,
                    "Maximum number of listeners per topic filter has been reached",
                ));
            }
            debug!(
                "Adding listener to existing subscription [topic filter: {}",
                topic_filter
            );
            listeners.insert(comp_listener);
            Ok(None)
        } else {
            // this fails if all subscription IDs have already been taken
            // [impl->dsn~utransport-registerlistener-error-resource-exhausted~1]
            if self.subscription_topics.len() == self.subscription_topics.capacity() {
                return Err(UStatus::fail_with_code(
                    UCode::RESOURCE_EXHAUSTED,
                    "Max number of subscriptions reached",
                ));
            }
            let mut hs = HashSet::new();
            hs.insert(comp_listener.clone());
            let subscription_id = self
                .subscription_topics
                .insert((topic_filter.to_string(), hs));
            let mut listeners = HashSet::new();
            listeners.insert(comp_listener);
            self.topic_listeners.insert(topic_filter, listeners);
            debug!("Added listener with subscribtion_id: {}", subscription_id);
            Ok(Some(u16::try_from(subscription_id).expect(NO_VALID_U16)))
        }
    }

    /// Checks if a given listener is the last one registered for a topic filter.
    ///
    /// # Returns
    ///
    /// `true` if the set of registered listeners for the topic filter only contains
    /// the given listener.
    pub(crate) fn is_last_listener(
        &self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> bool {
        self.topic_listeners
            .get(topic_filter)
            .is_some_and(|registered_listeners| {
                registered_listeners.len() == 1
                    && registered_listeners.contains(&ComparableListener::new(listener))
            })
    }

    /// Removes a listener for an MQTT topic filter.
    ///
    /// # Arguments
    /// * `topic_filter` - The topic filter to remove the listener for.
    /// * `listener` - Listener to remove from the topic subscription list.
    ///
    /// # Returns
    ///
    /// `true` if the listener had been registered for the topic filter.
    pub(crate) fn remove_listener(
        &mut self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> bool {
        let Some(registered_listeners) = self.topic_listeners.get_mut(topic_filter) else {
            return false;
        };

        if !registered_listeners.remove(&ComparableListener::new(listener.clone())) {
            return false;
        }
        let now_empty = registered_listeners.is_empty();

        // Remove from subscription_id mapping
        if let Some(sub_id) = self.find_subscription_id(topic_filter) {
            self.subscription_topics
                .get_mut(sub_id.into())
                .unwrap()
                .1
                .remove(&ComparableListener::new(listener));
            if now_empty {
                // find subscription ID for topic filter and release the subscription ID
                self.release_subscription_id(sub_id, topic_filter);
            }
        }
        true
    }

    /// Determines listeners registered for subscription IDs.
    pub(crate) fn determine_listeners_for_topic(&self, topic: &str) -> HashSet<ComparableListener> {
        let mut listeners_to_invoke = HashSet::new();
        self.topic_listeners
            .matches(topic)
            .for_each(|(_topic_filter, listeners)| {
                for listener in listeners {
                    listeners_to_invoke.insert(listener.to_owned());
                }
            });
        listeners_to_invoke
    }

    /// Determines listeners registered for subscription IDs.
    pub(crate) fn determine_listeners_for_subscription_id(
        &self,
        subscription_id: SubscriptionIdentifier,
    ) -> Option<&HashSet<ComparableListener>> {
        self.subscription_topics
            .get(subscription_id.into())
            .map(|topic_listener| &topic_listener.1)
    }
}

pub(crate) trait SubscribedTopicProvider: Send + Sync {
    fn get_subscribed_topics(&self) -> HashMap<SubscriptionIdentifier, String>;
}

impl SubscribedTopicProvider for RegisteredListeners {
    fn get_subscribed_topics(&self) -> HashMap<SubscriptionIdentifier, String> {
        self.subscription_topics
            .iter()
            .map(|(subscription_id, topic_filter)| {
                (
                    u16::try_from(subscription_id).expect(NO_VALID_U16),
                    topic_filter.0.clone(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use up_rust::MockUListener;

    use super::*;

    #[test]
    #[should_panic]
    fn test_panic_on_too_many_max_subs() {
        RegisteredListeners::new(u16::MAX, 5);
    }

    #[test]
    fn test_add_listener() {
        let topic_filter = "+/local_authority";
        let topic = "remote_authority/local_authority";
        let listener = Arc::new(MockUListener::new());
        let expected_listener = ComparableListener::new(listener.clone());
        let mut registered_listeners = RegisteredListeners::new(2, 2);

        assert!(registered_listeners
            .determine_listeners_for_topic(topic)
            .is_empty());

        let subscription_id = registered_listeners
            .add_listener(topic_filter, listener.clone())
            .expect("Failed to register listener")
            .expect("Did not create new subscription ID");

        let listeners =
            registered_listeners.determine_listeners_for_subscription_id(subscription_id);
        assert!(listeners.unwrap().len() == 1 && listeners.unwrap().contains(&expected_listener));
        let listeners = registered_listeners.determine_listeners_for_topic(topic);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));

        // [utest->dsn~utransport-registerlistener-idempotent~1]
        assert!(registered_listeners
            .add_listener(topic_filter, listener.clone())
            .expect("Failed to register listener")
            .is_none());
        assert!(registered_listeners
            .add_listener(topic_filter, listener.clone())
            .expect("Failed to register listener")
            .is_none());

        let listeners = registered_listeners.determine_listeners_for_topic(topic);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));

        // [utest->dsn~utransport-registerlistener-number-of-listeners~1]
        let other_listener = Arc::new(MockUListener::new());
        let expected_other_listener = ComparableListener::new(other_listener.clone());
        assert_ne!(expected_listener, expected_other_listener);
        assert!(registered_listeners
            .add_listener(topic_filter, other_listener)
            .expect("Failed to register listener")
            .is_none());
        let listeners = registered_listeners.determine_listeners_for_topic(topic);
        assert!(
            listeners.len() == 2
                && listeners.contains(&expected_listener)
                && listeners.contains(&expected_other_listener)
        );
    }

    #[test]
    fn test_add_listener_fails_for_exhausted_resources() {
        let topic_filter_1 = "source_1/local_authority";
        let topic_filter_2 = "source_2/local_authority";
        let listener = Arc::new(MockUListener::new());
        let listener_2 = Arc::new(MockUListener::new());
        // [utest->req~utransport-registerlistener-max-listeners~1]
        let mut registered_listeners = RegisteredListeners::new(1, 1);

        assert!(registered_listeners
            .determine_listeners_for_topic(topic_filter_1)
            .is_empty());

        let subscription_id = registered_listeners
            .add_listener(topic_filter_1, listener.clone())
            .expect("Failed to register listener")
            .expect("Did not create new subscription ID");

        let listeners =
            registered_listeners.determine_listeners_for_subscription_id(subscription_id);

        assert!(
            listeners.unwrap().len() == 1
                && listeners
                    .unwrap()
                    .contains(&ComparableListener::new(listener.clone())),
            "It should have been possible to register a single listener for one topic filter"
        );

        // [utest->dsn~utransport-registerlistener-error-resource-exhausted~1]
        assert!(registered_listeners
                .add_listener(topic_filter_1, listener_2.clone())
                .is_err_and(|err| err.get_code() == UCode::RESOURCE_EXHAUSTED),
            "It should not have been possible to register another listener for the same topic filter"
        );
        assert!(
            registered_listeners
                .add_listener(topic_filter_2, listener_2.clone())
                .is_err_and(|err| err.get_code() == UCode::RESOURCE_EXHAUSTED),
            "It should not have been possible to register a listener for another topic filter"
        );
    }

    #[test]
    // [utest->dsn~utransport-registerlistener-listener-reuse~1]
    fn test_add_listener_supports_multi_topic_registration() {
        let topic_1 = "remote_authority_1/local_authority";
        let topic_2 = "remote_authority_2/local_authority";

        let listener = Arc::new(MockUListener::new());
        let expected_listener = ComparableListener::new(listener.clone());
        let mut registered_listeners = RegisteredListeners::new(2, 2);

        assert!(registered_listeners
            .determine_listeners_for_topic(topic_1)
            .is_empty());
        assert!(registered_listeners
            .determine_listeners_for_topic(topic_2)
            .is_empty());

        let subscription_id_1 = registered_listeners
            .add_listener(topic_1, listener.clone())
            .expect("Failed to register listener")
            .expect("Did not create new subscription ID");
        let subscription_id_2 = registered_listeners
            .add_listener(topic_2, listener.clone())
            .expect("Failed to register listener")
            .expect("Did not create new subscription ID");

        let listeners =
            registered_listeners.determine_listeners_for_subscription_id(subscription_id_1);
        let _listener_id_2 =
            registered_listeners.determine_listeners_for_subscription_id(subscription_id_2);

        assert!(listeners.unwrap().len() == 1 && listeners.unwrap().contains(&expected_listener));
        let listeners = registered_listeners.determine_listeners_for_topic(topic_1);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));
        let listeners = registered_listeners.determine_listeners_for_topic(topic_2);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));
    }

    #[test]
    fn test_remove_listener() {
        let topic_filter = "+/local_authority";
        let listener_1 = Arc::new(MockUListener::new());
        let comparable_listener_1 = ComparableListener::new(listener_1.clone());
        let listener_2 = Arc::new(MockUListener::new());
        let comparable_listener_2 = ComparableListener::new(listener_2.clone());
        let mut registered_listeners = RegisteredListeners::default();

        let subscription_id = registered_listeners
            .add_listener(topic_filter, listener_1.clone())
            .expect("Failed to register listener 1")
            .expect("Did not create new subscription ID");

        assert!(registered_listeners
            .add_listener(topic_filter, listener_2.clone())
            .expect("Failed to register listener 2")
            .is_none());

        let listeners =
            registered_listeners.determine_listeners_for_subscription_id(subscription_id);
        assert!(
            listeners.unwrap().len() == 2
                && listeners.unwrap().contains(&comparable_listener_1)
                && listeners.unwrap().contains(&comparable_listener_2)
        );

        assert!(!registered_listeners.is_last_listener(topic_filter, listener_1.clone()));
        assert!(registered_listeners.remove_listener(topic_filter, listener_1.clone()));

        let listeners =
            registered_listeners.determine_listeners_for_subscription_id(subscription_id);
        println!("{}", listeners.unwrap().len());
        assert!(listeners.is_some_and(|l| l.len() == 1
            && !l.contains(&comparable_listener_1)
            && l.contains(&comparable_listener_2)));

        assert!(registered_listeners.is_last_listener(topic_filter, listener_2.clone()));
        assert!(registered_listeners.remove_listener(topic_filter, listener_2.clone()));

        assert!(registered_listeners
            .determine_listeners_for_subscription_id(subscription_id)
            .is_none());

        assert!(!registered_listeners.remove_listener(topic_filter, listener_2.clone()));
    }
}
