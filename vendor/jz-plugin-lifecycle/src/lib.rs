//! Shared lifecycle code for official Jacky plugins: the `commands --json`
//! surface, fresh install (`doctor --repair-install`) and the passive update
//! notice. Deliberately contains no auth code. Products consume it as a
//! vendored path dependency (`delivery/plugin-common-module.md` §1).

pub mod commands;
pub mod home;
pub mod install;
pub mod update;
