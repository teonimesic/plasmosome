use super::recipe::{PlatformLimits, RuntimeRecipe};
use serde_json::{Value, json};

pub(crate) mod fake {
    include!("../../tests/support/runtime_fake.rs");
}

pub(crate) fn artifact(path: &str, digit: char) -> Value {
    json!({"path": path, "sha256": digit.to_string().repeat(64)})
}

pub(crate) fn recipe_value(helper: &str) -> Value {
    let kernel_format = if std::env::consts::ARCH == "x86_64" {
        "elf"
    } else {
        "raw"
    };
    json!({
        "version": 1,
        "vcpus": 1,
        "memory_mib": 512,
        "kernel": artifact("/opt/cell/kernel", '1'),
        "kernel_format": kernel_format,
        "initramfs": artifact("/opt/cell/initramfs", '2'),
        "root_image": artifact("/opt/cell/root.raw", '3'),
        "writable_root": "/var/cell/c1/root.raw",
        "control_path": "/var/cell/c1/control.sock",
        "data_path": "/var/cell/c1/data.sock",
        "helper": artifact(helper, '4'),
        "libraries": [artifact("/opt/cell/lib/libkrun.1.dylib", '5')],
        "host_policy": artifact("/opt/cell/host.sb", '6'),
        "guest_policy": artifact("/opt/cell/guest.json", '7'),
        "architecture": std::env::consts::ARCH,
    })
}

pub(crate) fn recipe(value: &Value) -> RuntimeRecipe {
    let limits = PlatformLimits::host().expect("this host reports its limits");
    RuntimeRecipe::parse(format!("{value}\n").as_bytes(), &limits)
        .unwrap_or_else(|refusal| panic!("the test recipe is valid on this host: {refusal}"))
}
