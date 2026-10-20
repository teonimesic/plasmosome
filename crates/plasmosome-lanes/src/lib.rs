//! Test lanes that boot the pinned libkrun guest with no cell; a fixture no product path uses.

pub mod bundle;
pub mod config;
pub mod console;
pub mod cpio;
pub mod dyld;
pub mod ext4;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod guest_sys;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod krun;
pub mod probe;
pub mod run;
pub mod selfcheck;
