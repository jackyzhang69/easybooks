//! Shared plugin capabilities for Jacky official Portal plugins.
//!
//! Consumers construct a [`identity::PluginIdentity`] and call the module APIs.
//! No silent success: HTTP and delivery failures surface as typed errors or
//! explicit local-mirror outcomes.

pub mod auth;
pub mod doctor;
pub mod envelope;
pub mod home;
pub mod http;
pub mod human_action_ledger;
pub mod identity;
pub mod pair;
#[cfg(feature = "pair-a2a")]
pub mod pair_a2a;
#[cfg(feature = "pair-a2a")]
pub mod pair_a2a_session;
pub mod pair_host;
pub mod pair_receiver;
pub mod signals;
pub mod tell_jacky;

#[cfg(test)]
mod test_env;
