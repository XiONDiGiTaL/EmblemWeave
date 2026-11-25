//! The Vulkan devices the engine can choose between.
//!
//! The engine logs its physical devices in Vulkan's enumeration order and accepts
//! either a category (`discrete`, `integrated`) or part of a device name in
//! `[gfx] gpu` (see `runtime/docs/CONFIG.md`). The launcher writes the full name,
//! which avoids depending on the loader's order and makes a hybrid laptop's two
//! adapters distinguishable in the dropdown.
//!
//! The Performance Lab needs more than the picker: driver and API versions,
//! supported extensions, and the driver's heap budget when
//! `VK_EXT_memory_budget` is present. Those are read in the same enumeration so
//! the Lab never mixes a different loader instance's answer with the picker's.

/// One Vulkan physical device that the engine can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// `Discrete`, `Integrated`, `Virtual`, etc.
    pub kind: &'static str,
    pub vram_mb: u64,
    /// The driver advertises `VK_KHR_video_decode_av1`. This is the hardware
    /// AV1 decoder D3D11VA uses too; it is a capability report, not a promise
    /// that every driver build will decode every movie.
    pub av1_decode: bool,
}

/// One Vulkan memory heap and, when the driver exposes
/// `VK_EXT_memory_budget`, its current budget and process usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heap {
    pub size_mb: u64,
    pub device_local: bool,
    /// The OS/driver budget for this process on this heap. `None` means the
    /// extension is absent or the driver returned no figure; it is not zero.
    pub budget_mb: Option<u64>,
    /// This process's usage as reported by the extension.
    pub usage_mb: Option<u64>,
}

/// Everything the Lab reports about one physical device. `Device` stays the
/// small picker type so existing pages and capture fixtures do not change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub device: Device,
    pub vendor: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub api_version: String,
    pub driver: String,
    pub extensions: Vec<String>,
    pub heaps: Vec<Heap>,
}

impl Profile {
    /// The largest device-local heap, or the largest heap when the driver
    /// marks none as device-local.
    pub fn largest_device_local_heap(&self) -> Option<&Heap> {
        self.heaps
            .iter()
            .filter(|h| h.device_local)
            .max_by_key(|h| h.size_mb)
            .or_else(|| self.heaps.iter().max_by_key(|h| h.size_mb))
    }

    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.iter().any(|e| e.eq_ignore_ascii_case(name))
    }
}

/// Enumerate the Vulkan devices now. The loader is loaded dynamically so a
/// machine without Vulkan can still open the launcher and read the error page.
pub fn devices() -> Vec<Device> {
    profiles().into_iter().map(|p| p.device).collect()
}

/// Enumerate devices with the Lab's additional detail. This is a synchronous
/// scan and should be called from a cached/background path, not every frame.
pub fn profiles() -> Vec<Profile> {
    platform::profiles()
}

static CACHE: std::sync::OnceLock<Vec<Device>> = std::sync::OnceLock::new();
static SCAN: std::sync::Once = std::sync::Once::new();
static PROFILE_CACHE: std::sync::OnceLock<Vec<Profile>> = std::sync::OnceLock::new();
static PROFILE_SCAN: std::sync::Once = std::sync::Once::new();

/// The list for this launcher run, or `None` while the one scan still runs. The scan loads vulkan-1.dll and every ICD on its
/// own thread: on the UI thread, whose OpenGL context is current, loading NVIDIA's ICD crashed AMD's OpenGL driver at the
/// next `wglMakeCurrent` on a hybrid laptop.
pub fn cached() -> Option<&'static [Device]> {
    if let Some(list) = CACHE.get() {
        return Some(list.as_slice());
    }
    SCAN.call_once(|| {
        std::thread::spawn(|| {
            let _ = CACHE.set(devices());
        });
    });
    CACHE.get().map(Vec::as_slice)
}

/// The detailed list for the Lab, scanned once on a background thread for the
/// same reason as [`cached`]. `None` means the scan is still running.
pub fn cached_profiles() -> Option<&'static [Profile]> {
    if let Some(list) = PROFILE_CACHE.get() {
        return Some(list.as_slice());
    }
    PROFILE_SCAN.call_once(|| {
        std::thread::spawn(|| {
            let _ = PROFILE_CACHE.set(profiles());
        });
    });
    PROFILE_CACHE.get().map(Vec::as_slice)
}

/// For `--gpu-list`: render this list instead of scanning, so the Graphics page can be checked with a hybrid machine's two
/// adapters on a PC that has one, or with no Vulkan device at all. Must run before the first `cached()`; `false` means the
/// scan already owns the answer.
pub fn set_for_capture(devices: Vec<Device>) -> bool {
    CACHE.set(devices).is_ok()
}

#[cfg(windows)]
mod platform {
    use super::{Device, Heap, Profile};
    use std::ffi::c_void;
    use std::ptr;
    use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    type VkInstance = *mut c_void;
    type VkPhysicalDevice = *mut c_void;

    const VK_SUCCESS: i32 = 0;
    const VK_STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
    const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
    const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_PROPERTIES_2: u32 = 1000059006;
    const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_BUDGET_PROPERTIES_EXT: u32 = 1000237000;
    const VK_API_VERSION_1_0: u32 = 1 << 22;
    const VK_API_VERSION_1_3: u32 = (1 << 22) | (3 << 12);
    const VK_MEMORY_HEAP_DEVICE_LOCAL_BIT: u32 = 1;
    const VK_MAX_MEMORY_HEAPS: usize = 16;

    #[repr(C)]
    struct VkApplicationInfo {
        s_type: u32,
        p_next: *const c_void,
        p_application_name: *const u8,
        application_version: u32,
        p_engine_name: *const u8,
        engine_version: u32,
        api_version: u32,
    }

    #[repr(C)]
    struct VkInstanceCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: u32,
        p_application_info: *const VkApplicationInfo,
        enabled_layer_count: u32,
        pp_enabled_layer_names: *const *const u8,
        enabled_extension_count: u32,
        pp_enabled_extension_names: *const *const u8,
    }

    /// Only the fields before `limits` are read. `rest` keeps the C buffer at
    /// least as large as the driver's full `VkPhysicalDeviceProperties`.
    #[repr(C)]
    struct VkPhysicalDeviceProperties {
        api_version: u32,
        driver_version: u32,
        vendor_id: u32,
        device_id: u32,
        device_type: u32,
        device_name: [u8; 256],
        pipeline_cache_uuid: [u8; 16],
        rest: [u8; 1024],
    }

    #[derive(Clone, Copy)]
    #[repr(C)]
    struct VkExtensionProperties {
        extension_name: [u8; 256],
        spec_version: u32,
    }

    #[repr(C)]
    struct VkMemoryType {
        property_flags: u32,
        heap_index: u32,
    }

    #[repr(C)]
    struct VkMemoryHeap {
        size: u64,
        flags: u32,
    }

    #[repr(C)]
    struct VkPhysicalDeviceMemoryProperties {
        memory_type_count: u32,
        memory_types: [VkMemoryType; 32],
        memory_heap_count: u32,
        memory_heaps: [VkMemoryHeap; VK_MAX_MEMORY_HEAPS],
    }

    #[repr(C)]
    struct VkPhysicalDeviceMemoryBudgetProperties {
        s_type: u32,
        p_next: *mut c_void,
        heap_budget: [u64; VK_MAX_MEMORY_HEAPS],
        heap_usage: [u64; VK_MAX_MEMORY_HEAPS],
    }

    #[repr(C)]
    struct VkPhysicalDeviceMemoryProperties2 {
        s_type: u32,
        p_next: *mut c_void,
        memory_properties: VkPhysicalDeviceMemoryProperties,
    }

    type CreateInstance = unsafe extern "system" fn(
        *const VkInstanceCreateInfo,
        *const c_void,
        *mut VkInstance,
    ) -> i32;
    type DestroyInstance = unsafe extern "system" fn(VkInstance, *const c_void);
    type EnumeratePhysicalDevices =
        unsafe extern "system" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32;
    type GetPhysicalDeviceProperties =
        unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDeviceProperties);
    type GetPhysicalDeviceMemoryProperties =
        unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties);
    type GetPhysicalDeviceMemoryProperties2 = unsafe extern "system" fn(
        VkPhysicalDevice,
        *mut VkPhysicalDeviceMemoryProperties2,
    );
    type EnumerateDeviceExtensionProperties = unsafe extern "system" fn(
        VkPhysicalDevice,
        *const u8,
        *mut u32,
        *mut VkExtensionProperties,
    ) -> i32;

    fn c_name(bytes: &[u8]) -> String {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).trim().to_string()
    }

    unsafe fn extensions(
        device: VkPhysicalDevice,
        enumerate: Option<EnumerateDeviceExtensionProperties>,
    ) -> Vec<String> {
        let Some(enumerate) = enumerate else {
            return Vec::new();
        };
        let mut count = 0u32;
        if enumerate(device, ptr::null(), &mut count, ptr::null_mut()) != VK_SUCCESS || count == 0 {
            return Vec::new();
        }
        let mut props: Vec<VkExtensionProperties> = vec![std::mem::zeroed(); count as usize];
        if enumerate(device, ptr::null(), &mut count, props.as_mut_ptr()) != VK_SUCCESS {
            return Vec::new();
        }
        props.truncate(count as usize);
        let mut out: Vec<String> = props.iter().map(|p| c_name(&p.extension_name)).filter(|n| !n.is_empty()).collect();
        out.sort();
        out.dedup();
        out
    }

    fn kind(device_type: u32) -> &'static str {
        match device_type {
            1 => "Integrated",
            2 => "Discrete",
            3 => "Virtual",
            4 => "CPU",
            _ => "Other",
        }
    }

    fn vendor_name(id: u32) -> String {
        match id {
            0x1002 => "AMD",
            0x1010 => "Imagination",
            0x10DE => "NVIDIA",
            0x13B5 => "ARM",
            0x5143 => "Qualcomm",
            0x8086 => "Intel",
            _ => "Vendor",
        }
        .to_string()
    }

    fn api_version(v: u32) -> String {
        format!("Vulkan {}.{}.{}", v >> 22, (v >> 12) & 0x3ff, v & 0xfff)
    }

    /// Vulkan's driver-version packing differs by vendor; the raw number is
    /// kept when the packing is not known so the Lab never invents a version.
    fn driver_version(vendor_id: u32, v: u32) -> String {
        match vendor_id {
            0x10DE => format!("{}.{}.{}.{}", (v >> 22) & 0x3ff, (v >> 14) & 0xff, (v >> 6) & 0xff, v & 0x3f),
            0x8086 => format!("{}.{}.{}", (v >> 14) & 0x3ffff, (v >> 6) & 0xff, v & 0x3f),
            0x1002 => {
                // AMD drivers use the Windows display-driver version in the
                // advertised field, packed as 15/12/12 bits.
                format!("{}.{}.{}", v >> 17, (v >> 5) & 0xfff, (v & 0x1f) << 1)
            }
            _ => format!("0x{v:08X}"),
        }
    }

    fn mib(bytes: u64) -> u64 {
        bytes / (1024 * 1024)
    }

    pub fn profiles() -> Vec<Profile> {
        let dll: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
        unsafe {
            let lib = LoadLibraryW(dll.as_ptr());
            if lib.is_null() {
                return Vec::new();
            }
            let out = read_profiles(lib);
            FreeLibrary(lib);
            out
        }
    }

    unsafe fn read_profiles(lib: HMODULE) -> Vec<Profile> {
        let sym = |name: &[u8]| GetProcAddress(lib, name.as_ptr()).map(|f| f as *const c_void);
        let Some(create) = sym(b"vkCreateInstance\0") else {
            return Vec::new();
        };
        let Some(destroy) = sym(b"vkDestroyInstance\0") else {
            return Vec::new();
        };
        let Some(enumerate) = sym(b"vkEnumeratePhysicalDevices\0") else {
            return Vec::new();
        };
        let Some(properties) = sym(b"vkGetPhysicalDeviceProperties\0") else {
            return Vec::new();
        };
        let Some(memory) = sym(b"vkGetPhysicalDeviceMemoryProperties\0") else {
            return Vec::new();
        };
        let extension_fn = sym(b"vkEnumerateDeviceExtensionProperties\0")
            .map(|f| std::mem::transmute::<*const c_void, EnumerateDeviceExtensionProperties>(f));
        let query_memory = sym(b"vkGetPhysicalDeviceMemoryProperties2\0")
            .or_else(|| sym(b"vkGetPhysicalDeviceMemoryProperties2KHR\0"))
            .map(|f| std::mem::transmute::<*const c_void, GetPhysicalDeviceMemoryProperties2>(f));
        let create: CreateInstance = std::mem::transmute(create);
        let destroy: DestroyInstance = std::mem::transmute(destroy);
        let enumerate: EnumeratePhysicalDevices = std::mem::transmute(enumerate);
        let properties: GetPhysicalDeviceProperties = std::mem::transmute(properties);
        let memory: GetPhysicalDeviceMemoryProperties = std::mem::transmute(memory);

        let app = VkApplicationInfo {
            s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
            p_next: ptr::null(),
            p_application_name: b"EmblemWeave\0".as_ptr(),
            application_version: 0,
            p_engine_name: b"EmblemWeave\0".as_ptr(),
            engine_version: 0,
            api_version: VK_API_VERSION_1_0,
        };
        let info = VkInstanceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            p_application_info: &app,
            enabled_layer_count: 0,
            pp_enabled_layer_names: ptr::null(),
            enabled_extension_count: 0,
            pp_enabled_extension_names: ptr::null(),
        };
        let mut instance: VkInstance = ptr::null_mut();
        if create(&info, ptr::null(), &mut instance) != VK_SUCCESS || instance.is_null() {
            return Vec::new();
        }

        let mut count = 0u32;
        let mut out = Vec::new();
        if enumerate(instance, &mut count, ptr::null_mut()) == VK_SUCCESS && count > 0 {
            let mut physical = vec![ptr::null_mut(); count as usize];
            if enumerate(instance, &mut count, physical.as_mut_ptr()) == VK_SUCCESS {
                physical.truncate(count as usize);
                for dev in physical {
                    let mut props: VkPhysicalDeviceProperties = std::mem::zeroed();
                    properties(dev, &mut props);
                    if props.api_version < VK_API_VERSION_1_3 || props.device_type == 4 {
                        continue;
                    }
                    let name = c_name(&props.device_name);
                    if name.is_empty() {
                        continue;
                    }
                    let mut mem: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
                    memory(dev, &mut mem);
                    let ext = extensions(dev, extension_fn);
                    let budget_capable = ext.iter().any(|e| e.eq_ignore_ascii_case("VK_EXT_memory_budget"));
                    let mut budget: VkPhysicalDeviceMemoryBudgetProperties = std::mem::zeroed();
                    let mut budget_ok = false;
                    if budget_capable {
                        if let Some(query) = query_memory {
                            budget.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_BUDGET_PROPERTIES_EXT;
                            let mut mp: VkPhysicalDeviceMemoryProperties2 = std::mem::zeroed();
                            mp.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_PROPERTIES_2;
                            mp.p_next = &mut budget as *mut _ as *mut c_void;
                            query(dev, &mut mp);
                            budget_ok = true;
                        }
                    }
                    let heap_count = mem.memory_heap_count.min(VK_MAX_MEMORY_HEAPS as u32) as usize;
                    let mut heaps = Vec::with_capacity(heap_count);
                    for (i, heap) in mem.memory_heaps[..heap_count].iter().enumerate() {
                        heaps.push(Heap {
                            size_mb: mib(heap.size),
                            device_local: heap.flags & VK_MEMORY_HEAP_DEVICE_LOCAL_BIT != 0,
                            budget_mb: budget_ok.then(|| mib(budget.heap_budget[i])),
                            usage_mb: budget_ok.then(|| mib(budget.heap_usage[i])),
                        });
                    }
                    let vram = heaps
                        .iter()
                        .filter(|h| h.device_local)
                        .map(|h| h.size_mb)
                        .max()
                        .unwrap_or(0);
                    let av1_decode = ext.iter().any(|e| e.eq_ignore_ascii_case("VK_KHR_video_decode_av1"));
                    let vendor_id = props.vendor_id;
                    out.push(Profile {
                        device: Device {
                            name,
                            kind: kind(props.device_type),
                            vram_mb: vram,
                            av1_decode,
                        },
                        vendor: vendor_name(vendor_id),
                        vendor_id,
                        device_id: props.device_id,
                        api_version: api_version(props.api_version),
                        driver: driver_version(vendor_id, props.driver_version),
                        extensions: ext,
                        heaps,
                    });
                }
            }
        }
        destroy(instance, ptr::null());
        out
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Profile;

    pub fn profiles() -> Vec<Profile> {
        Vec::new()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Lists the Vulkan devices on this machine. Ignored by default so the unit
    /// suite does not depend on a GPU; run with `--ignored --nocapture` on a
    /// hybrid machine to see the same list the Graphics page shows.
    #[test]
    #[ignore]
    fn lists_vulkan_devices() {
        let devices = devices();
        for d in &devices {
            println!("{} ({}, {} MiB)", d.name, d.kind, d.vram_mb);
        }
        assert!(!devices.is_empty(), "no Vulkan 1.3 device found");
    }

    #[test]
    #[ignore]
    fn lists_vulkan_profile_detail() {
        for p in profiles() {
            println!("{} / {} / {} / driver {}", p.device.name, p.vendor, p.api_version, p.driver);
            println!("  {} extensions", p.extensions.len());
            for h in &p.heaps {
                println!(
                    "  {} MiB{} budget {:?} use {:?}",
                    h.size_mb,
                    if h.device_local { " device-local" } else { "" },
                    h.budget_mb,
                    h.usage_mb
                );
            }
        }
    }
}
