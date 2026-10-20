use super::recipe::{Architecture, PlatformLimits};
use serde_json::{Value, json};

pub(crate) const LIMITS: PlatformLimits =
    PlatformLimits::new(Architecture::Aarch64, 8, 16_384, 103);

pub(crate) fn digest(digit: char) -> String {
    digit.to_string().repeat(64)
}

pub(crate) fn artifact(path: &str, digit: char) -> Value {
    json!({"path": path, "sha256": digest(digit)})
}

pub(crate) fn valid_recipe() -> Value {
    json!({
        "version": 1,
        "vcpus": 2,
        "memory_mib": 2048,
        "kernel": artifact("/opt/cell/kernel", '1'),
        "kernel_format": "raw",
        "initramfs": artifact("/opt/cell/initramfs", '2'),
        "root_image": artifact("/opt/cell/root.raw", '3'),
        "writable_root": "/var/cell/c1/root.raw",
        "control_path": "/var/cell/c1/control.sock",
        "data_path": "/var/cell/c1/data.sock",
        "helper": artifact("/opt/cell/krun-helper", '4'),
        "libraries": [artifact("/opt/cell/lib/libkrun.1.dylib", '5')],
        "host_policy": artifact("/opt/cell/host.sb", '6'),
        "guest_policy": artifact("/opt/cell/guest.json", '7'),
        "architecture": "aarch64",
    })
}
