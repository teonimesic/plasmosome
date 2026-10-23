#![cfg(feature = "vmm-helper")]

#[path = "../src/bin/plasmosome-vmm/diagnostics.rs"]
mod diagnostics;
#[path = "../src/bin/plasmosome-vmm/loader.rs"]
mod loader;
#[path = "../src/bin/plasmosome-vmm/pre_native.rs"]
mod pre_native;

const _: fn() -> Result<(), plasmosome_membrane::runtime::vmm_helper::HelperRefusal> =
    loader::refuse_loader_inputs;
