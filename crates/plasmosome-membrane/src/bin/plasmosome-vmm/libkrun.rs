use plasmosome_membrane::runtime::vmm_helper::KrunCall;
use std::ffi::c_char;

#[allow(unsafe_code)]
unsafe extern "C" {
    /// Pinned libkrun v1.19.4 include/libkrun.h:66; descriptor 2 stays open.
    fn krun_init_log(fd: i32, level: u32, style: u32, options: u32) -> i32;
    /// Header:74; creates a context, a negative return is an error.
    fn krun_create_ctx() -> i32;
    /// Header:98; context belongs to this process.
    fn krun_set_vm_config(ctx: u32, cpus: u8, mib: u32) -> i32;
    /// Header:874-878; every pointer names a live terminated string during the call.
    fn krun_set_kernel(
        ctx: u32,
        kernel: *const c_char,
        format: u32,
        initramfs: *const c_char,
        cmdline: *const c_char,
    ) -> i32;
    /// Header:278-284; strings stay live and C bool matches Rust bool.
    fn krun_add_disk3(
        ctx: u32,
        id: *const c_char,
        path: *const c_char,
        format: u32,
        read_only: bool,
        direct_io: bool,
        sync: u32,
    ) -> i32;
    /// Header:1122; context belongs to this process.
    fn krun_disable_implicit_init(ctx: u32) -> i32;
    /// Header:1107; context belongs to this process.
    fn krun_disable_implicit_console(ctx: u32) -> i32;
    /// Header:1213; context belongs to this process.
    fn krun_disable_implicit_vsock(ctx: u32) -> i32;
    /// Header:952; flags are the pinned plan's TSI flags.
    fn krun_add_vsock(ctx: u32, flags: u32) -> i32;
    /// Header:892; pointer is a live NULL-terminated array, not NULL itself.
    fn krun_set_env(ctx: u32, env: *const *const c_char) -> i32;
    /// Header:605; pointer is a live NULL-terminated array, not NULL itself.
    fn krun_set_port_map(ctx: u32, ports: *const *const c_char) -> i32;
    /// Header:929-932; path stays live during the call; C bool matches Rust bool.
    fn krun_add_vsock_port2(ctx: u32, port: u32, path: *const c_char, listen: bool) -> i32;
    /// Header:1385; consumes the context and returns only on launch failure.
    fn krun_start_enter(ctx: u32) -> i32;
}

#[allow(unsafe_code)]
pub(super) fn call(step: &KrunCall, context: Option<u32>) -> i32 {
    let ctx = context.unwrap_or(0);
    let empty = [std::ptr::null()];
    // SAFETY: run_plan supplies a live context after create and stops on the first error.
    // The plan owns every CString through all calls; libkrun copies the strings it retains.
    // Empty arrays are non-null and terminated. The pinned header declares the exact types
    // above; init_log targets the diagnostic pipe and start consumes the context only once.
    unsafe {
        match step {
            KrunCall::InitLog {
                target_fd,
                level,
                style,
                options,
            } => krun_init_log(*target_fd, *level, *style, *options),
            KrunCall::CreateCtx => krun_create_ctx(),
            KrunCall::SetVmConfig { vcpus, ram_mib } => krun_set_vm_config(ctx, *vcpus, *ram_mib),
            KrunCall::SetKernel {
                kernel,
                format,
                initramfs,
                cmdline,
            } => krun_set_kernel(
                ctx,
                kernel.as_ptr(),
                *format,
                initramfs.as_ptr(),
                cmdline.as_ptr(),
            ),
            KrunCall::AddDisk3 {
                block_id,
                path,
                format,
                read_only,
                direct_io,
                sync_mode,
            } => krun_add_disk3(
                ctx,
                block_id.as_ptr(),
                path.as_ptr(),
                *format,
                *read_only,
                *direct_io,
                *sync_mode,
            ),
            KrunCall::DisableImplicitInit => krun_disable_implicit_init(ctx),
            KrunCall::DisableImplicitConsole => krun_disable_implicit_console(ctx),
            KrunCall::DisableImplicitVsock => krun_disable_implicit_vsock(ctx),
            KrunCall::AddVsock { tsi_features } => krun_add_vsock(ctx, *tsi_features),
            KrunCall::SetEnvEmpty => krun_set_env(ctx, empty.as_ptr()),
            KrunCall::SetPortMapEmpty => krun_set_port_map(ctx, empty.as_ptr()),
            KrunCall::AddVsockPort2 { port, path, listen } => {
                krun_add_vsock_port2(ctx, *port, path.as_ptr(), *listen)
            }
            KrunCall::StartEnter => krun_start_enter(ctx),
        }
    }
}
