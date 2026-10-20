use std::ffi::{CStr, CString, OsString, c_char, c_int, c_void};
use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// The command line spec 001 §4.2 fixes for the guest kernel.
pub const CMDLINE: &str = "rdinit=/init panic=-1";

const KRUN_LOG_LEVEL_WARN: u32 = 2;
const KRUN_LOG_STYLE_NEVER: u32 = 2;
const KRUN_LOG_OPTION_NO_ENV: u32 = 1;
const KRUN_KERNEL_FORMAT_RAW: u32 = 0;
const KRUN_DISK_FORMAT_RAW: u32 = 0;
const KRUN_SYNC_FULL: u32 = 2;

/// A libkrun or loader call that failed, naming the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrunError(pub String);

impl fmt::Display for KrunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KrunError {}

type InitLog = unsafe extern "C" fn(c_int, u32, u32, u32) -> i32;
type CreateCtx = unsafe extern "C" fn() -> i32;
type SetVmConfig = unsafe extern "C" fn(u32, u8, u32) -> i32;
type SetKernel = unsafe extern "C" fn(u32, *const c_char, u32, *const c_char, *const c_char) -> i32;
type AddDisk3 =
    unsafe extern "C" fn(u32, *const c_char, *const c_char, u32, bool, bool, u32) -> i32;
type ContextOnly = unsafe extern "C" fn(u32) -> i32;
type AddVirtiofs4 = unsafe extern "C" fn(u32, *const c_char, *const c_char, u64, bool, u32) -> i32;
type SetConsoleOutput = unsafe extern "C" fn(u32, *const c_char) -> i32;

struct Symbols {
    init_log: InitLog,
    create_ctx: CreateCtx,
    set_vm_config: SetVmConfig,
    set_kernel: SetKernel,
    add_disk3: AddDisk3,
    disable_implicit_init: ContextOnly,
    disable_implicit_vsock: ContextOnly,
    add_virtiofs4: AddVirtiofs4,
    set_console_output: SetConsoleOutput,
    start_enter: ContextOnly,
}

/// The bundle's libkrun, loaded by absolute path and never unloaded.
pub struct Libkrun {
    path: PathBuf,
    symbols: Symbols,
}

/// A libkrun configuration context that `start_enter` has not consumed.
pub struct Context<'a> {
    lib: &'a Libkrun,
    id: u32,
}

fn c_string(what: &str, bytes: &[u8]) -> Result<CString, KrunError> {
    CString::new(bytes).map_err(|_| KrunError(format!("{what} holds a NUL byte")))
}

fn c_path(what: &str, path: &Path) -> Result<CString, KrunError> {
    c_string(what, path.as_os_str().as_bytes())
}

fn last_dlerror() -> String {
    // SAFETY: dlerror takes no arguments and returns either null or a pointer to a
    // NUL-terminated message owned by the loader, valid until the next dl* call on this thread.
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        return "no loader message".to_string();
    }
    // SAFETY: the pointer is non-null and points to the loader's NUL-terminated message, which
    // is copied out before any other dl* call.
    unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .to_string()
}

/// Look up `name` in `handle` as a function of type `T`.
///
/// # Safety
///
/// `handle` must be a live handle from `dlopen`, and `T` must be the `unsafe extern "C" fn`
/// type that the pinned `libkrun.h` declares for `name`.
unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> Result<T, KrunError> {
    // SAFETY: the caller guarantees `handle` is live, and `name` is NUL-terminated.
    let address = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if address.is_null() {
        return Err(KrunError(format!(
            "dlsym({}) failed: {}",
            name.to_string_lossy(),
            last_dlerror()
        )));
    }
    assert_eq!(size_of::<T>(), size_of::<*mut c_void>());
    // SAFETY: the caller guarantees `T` is this symbol's function pointer type, and on macOS a
    // function pointer and a data pointer have the same size and representation.
    Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&address) })
}

fn checked(call: &str, rc: i32) -> Result<(), KrunError> {
    if rc == 0 {
        Ok(())
    } else {
        Err(KrunError(format!("{call} returned {rc}")))
    }
}

impl Libkrun {
    /// Load the libkrun at the absolute `path` and resolve every function `lane-vmm` calls.
    /// The library's initializers run here, so verify its bytes first.
    pub fn open(path: &Path) -> Result<Libkrun, KrunError> {
        let c_name = c_path("the libkrun path", path)?;
        // SAFETY: `c_name` is a NUL-terminated absolute path. Loading runs the library's
        // initializers; the caller verified the file against the bundle's SHA256SUMS first.
        let handle = unsafe { libc::dlopen(c_name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(KrunError(format!(
                "dlopen({}) failed: {}",
                path.display(),
                last_dlerror()
            )));
        }
        // SAFETY: `handle` was just returned by dlopen and is never closed. Each type alias
        // matches the declaration of the same name in libkrun v1.19.4's include/libkrun.h.
        let symbols = unsafe {
            Symbols {
                init_log: symbol(handle, c"krun_init_log")?,
                create_ctx: symbol(handle, c"krun_create_ctx")?,
                set_vm_config: symbol(handle, c"krun_set_vm_config")?,
                set_kernel: symbol(handle, c"krun_set_kernel")?,
                add_disk3: symbol(handle, c"krun_add_disk3")?,
                disable_implicit_init: symbol(handle, c"krun_disable_implicit_init")?,
                disable_implicit_vsock: symbol(handle, c"krun_disable_implicit_vsock")?,
                add_virtiofs4: symbol(handle, c"krun_add_virtiofs4")?,
                set_console_output: symbol(handle, c"krun_set_console_output")?,
                start_enter: symbol(handle, c"krun_start_enter")?,
            }
        };
        Ok(Libkrun {
            path: path.to_path_buf(),
            symbols,
        })
    }

    /// The path the library was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `krun_init_log(2, KRUN_LOG_LEVEL_WARN, KRUN_LOG_STYLE_NEVER, KRUN_LOG_OPTION_NO_ENV)`,
    /// the only logging spec 001 §4.2 allows.
    pub fn init_log(&self) -> Result<(), KrunError> {
        // SAFETY: the pointer came from dlsym with the header's signature, and the arguments
        // are plain integers.
        let rc = unsafe {
            (self.symbols.init_log)(
                2,
                KRUN_LOG_LEVEL_WARN,
                KRUN_LOG_STYLE_NEVER,
                KRUN_LOG_OPTION_NO_ENV,
            )
        };
        checked("krun_init_log", rc)
    }

    /// `krun_create_ctx`. A negative return refuses.
    pub fn create_ctx(&self) -> Result<Context<'_>, KrunError> {
        // SAFETY: the pointer came from dlsym with the header's signature; it takes nothing.
        let id = unsafe { (self.symbols.create_ctx)() };
        u32::try_from(id)
            .map(|id| Context { lib: self, id })
            .map_err(|_| KrunError(format!("krun_create_ctx returned {id}")))
    }
}

impl Context<'_> {
    /// `krun_set_vm_config(ctx, vcpus, memory_mib)`.
    pub fn set_vm_config(&self, vcpus: u8, memory_mib: u32) -> Result<(), KrunError> {
        // SAFETY: the pointer came from dlsym with the header's signature, `self.id` is a live
        // context, and the other arguments are plain integers.
        let rc = unsafe { (self.lib.symbols.set_vm_config)(self.id, vcpus, memory_mib) };
        checked("krun_set_vm_config", rc)
    }

    /// `krun_set_kernel(ctx, kernel, KRUN_KERNEL_FORMAT_RAW, initramfs, CMDLINE)`.
    pub fn set_kernel(&self, kernel: &Path, initramfs: &Path) -> Result<(), KrunError> {
        let kernel = c_path("the kernel path", kernel)?;
        let initramfs = c_path("the initramfs path", initramfs)?;
        let cmdline = c_string("the command line", CMDLINE.as_bytes())?;
        // SAFETY: the pointer came from dlsym with the header's signature, `self.id` is a live
        // context, and the three strings are NUL-terminated and outlive the call; libkrun
        // copies them before it returns.
        let rc = unsafe {
            (self.lib.symbols.set_kernel)(
                self.id,
                kernel.as_ptr(),
                KRUN_KERNEL_FORMAT_RAW,
                initramfs.as_ptr(),
                cmdline.as_ptr(),
            )
        };
        checked("krun_set_kernel", rc)
    }

    /// `krun_add_disk3(ctx, "root", image, KRUN_DISK_FORMAT_RAW, read_only, false,
    /// KRUN_SYNC_FULL)`.
    pub fn add_root_disk(&self, image: &Path, read_only: bool) -> Result<(), KrunError> {
        let image = c_path("the root image path", image)?;
        // SAFETY: the pointer came from dlsym with the header's signature, `self.id` is a live
        // context, and both strings are NUL-terminated and outlive the call; libkrun copies
        // them before it returns.
        let rc = unsafe {
            (self.lib.symbols.add_disk3)(
                self.id,
                c"root".as_ptr(),
                image.as_ptr(),
                KRUN_DISK_FORMAT_RAW,
                read_only,
                false,
                KRUN_SYNC_FULL,
            )
        };
        checked("krun_add_disk3", rc)
    }

    /// `krun_disable_implicit_init(ctx)`.
    pub fn disable_implicit_init(&self) -> Result<(), KrunError> {
        // SAFETY: the pointer came from dlsym with the header's signature and `self.id` is a
        // live context.
        let rc = unsafe { (self.lib.symbols.disable_implicit_init)(self.id) };
        checked("krun_disable_implicit_init", rc)
    }

    /// `krun_disable_implicit_vsock(ctx)`: no vsock device at all.
    pub fn disable_implicit_vsock(&self) -> Result<(), KrunError> {
        // SAFETY: the pointer came from dlsym with the header's signature and `self.id` is a
        // live context.
        let rc = unsafe { (self.lib.symbols.disable_implicit_vsock)(self.id) };
        checked("krun_disable_implicit_vsock", rc)
    }

    /// `krun_add_virtiofs4(ctx, tag, path, shm_size, read_only, semantics)`.
    pub fn add_virtiofs4(
        &self,
        tag: &str,
        path: &Path,
        shm_size: u64,
        read_only: bool,
        semantics: u32,
    ) -> Result<(), KrunError> {
        let tag = c_string("a share tag", tag.as_bytes())?;
        let path = c_path("a share path", path)?;
        // SAFETY: the pointer came from dlsym with the header's signature, `self.id` is a live
        // context, and both strings are NUL-terminated and outlive the call; libkrun copies
        // them before it returns.
        let rc = unsafe {
            (self.lib.symbols.add_virtiofs4)(
                self.id,
                tag.as_ptr(),
                path.as_ptr(),
                shm_size,
                read_only,
                semantics,
            )
        };
        checked("krun_add_virtiofs4", rc)
    }

    /// `krun_set_console_output(ctx, path)`: the implicit console writes to `path`.
    pub fn set_console_output(&self, path: &Path) -> Result<(), KrunError> {
        let path = c_path("the console path", path)?;
        // SAFETY: the pointer came from dlsym with the header's signature, `self.id` is a live
        // context, and the string is NUL-terminated and outlives the call; libkrun copies it
        // before it returns.
        let rc = unsafe { (self.lib.symbols.set_console_output)(self.id, path.as_ptr()) };
        checked("krun_set_console_output", rc)
    }

    /// `krun_start_enter(ctx)`. It returns only on failure; when the guest powers off,
    /// libkrun ends the process itself.
    pub fn start_enter(self) -> KrunError {
        // SAFETY: the pointer came from dlsym with the header's signature, and `self.id` is a
        // live context that this call consumes, as `self` is taken by value.
        let rc = unsafe { (self.lib.symbols.start_enter)(self.id) };
        KrunError(format!("krun_start_enter returned {rc}"))
    }
}

/// The path of every image the loader has mapped into this process, the main executable first.
pub fn loaded_images() -> Vec<PathBuf> {
    (0.._dyld_image_count())
        .filter_map(|index| {
            let name = _dyld_get_image_name(index);
            if name.is_null() {
                return None;
            }
            // SAFETY: `name` is non-null, so it points to a NUL-terminated path the loader owns
            // for as long as the image stays loaded, and no image is unloaded while it is copied.
            let bytes = unsafe { CStr::from_ptr(name) }.to_bytes().to_vec();
            Some(PathBuf::from(OsString::from_vec(bytes)))
        })
        .collect()
}

unsafe extern "C" {
    /// dyld's count of mapped images. It takes nothing and only reads the loader's list.
    safe fn _dyld_image_count() -> u32;
    /// dyld's path for image `index`: null when `index` is not below the count, otherwise a
    /// NUL-terminated string the loader owns while that image stays loaded.
    safe fn _dyld_get_image_name(index: u32) -> *const c_char;
    /// libsystem_sandbox's SBPL entry point, deprecated but present on macOS 26. With `flags`
    /// 0, `profile` is SBPL source text. On failure it returns nonzero and stores a message in
    /// `*errorbuf` that `sandbox_free_error` releases; it reads `profile` only during the call.
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    /// Releases a message `sandbox_init` stored; `errorbuf` must be that pointer, freed once.
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// Apply the SBPL `profile` to this process. It cannot be lifted afterwards.
pub fn confine(profile: &str) -> Result<(), KrunError> {
    let profile = c_string("the SBPL profile", profile.as_bytes())?;
    let mut error: *mut c_char = std::ptr::null_mut();
    // SAFETY: `profile` is NUL-terminated and outlives the call, and `error` is a valid place
    // for sandbox_init to store a message pointer.
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
    if rc == 0 {
        return Ok(());
    }
    let message = if error.is_null() {
        "no message".to_string()
    } else {
        // SAFETY: sandbox_init stored a NUL-terminated message here, which is copied out and
        // then freed exactly once with the function that pairs with it.
        let text = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .to_string();
        // SAFETY: `error` is the message pointer sandbox_init stored and is not used again.
        unsafe { sandbox_free_error(error) };
        text
    };
    Err(KrunError(format!("sandbox_init returned {rc}: {message}")))
}
