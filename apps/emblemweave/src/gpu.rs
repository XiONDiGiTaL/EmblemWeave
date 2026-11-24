//! The Vulkan devices the engine can choose between.
//!
//! The engine logs its physical devices in Vulkan's enumeration order and accepts
//! either a category (`discrete`, `integrated`) or part of a device name in
//! `[gfx] gpu` (see `runtime/docs/CONFIG.md`). The launcher writes the full name,
//! which avoids depending on the loader's order and makes a hybrid laptop's two
//! adapters distinguishable in the dropdown.

/// One Vulkan physical device that the engine can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// `Discrete`, `Integrated`, `Virtual`, etc.
    pub kind: &'static str,
    pub vram_mb: u64,
}

/// Enumerate the Vulkan devices now. The loader is loaded dynamically so a
/// machine without Vulkan can still open the launcher and read the error page.
pub fn devices() -> Vec<Device> {
    platform::devices()
}

/// The list for this launcher run, or `None` while the one scan still runs. The scan loads vulkan-1.dll and every ICD on its
/// own thread: on the UI thread, whose OpenGL context is current, loading NVIDIA's ICD crashed AMD's OpenGL driver at the
/// next `wglMakeCurrent` on a hybrid laptop.
pub fn cached() -> Option<&'static [Device]> {
    static CACHE: std::sync::OnceLock<Vec<Device>> = std::sync::OnceLock::new();
    static SCAN: std::sync::Once = std::sync::Once::new();
    SCAN.call_once(|| {
        std::thread::spawn(|| {
            let _ = CACHE.set(devices());
        });
    });
    CACHE.get().map(Vec::as_slice)
}

#[cfg(windows)]
mod platform {
    use super::Device;
    use std::ffi::c_void;
    use std::ptr;
    use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    type VkInstance = *mut c_void;
    type VkPhysicalDevice = *mut c_void;

    const VK_SUCCESS: i32 = 0;
    const VK_STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
    const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
    const VK_API_VERSION_1_0: u32 = 1 << 22;
    const VK_API_VERSION_1_3: u32 = (1 << 22) | (3 << 12);
    const VK_MEMORY_HEAP_DEVICE_LOCAL_BIT: u32 = 1;

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
        memory_heaps: [VkMemoryHeap; 16],
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

    fn kind(device_type: u32) -> &'static str {
        match device_type {
            1 => "Integrated",
            2 => "Discrete",
            3 => "Virtual",
            4 => "CPU",
            _ => "Other",
        }
    }

    pub fn devices() -> Vec<Device> {
        // Vulkan is not a link-time dependency: a machine without the loader
        // still opens the launcher, and the page falls back to the categories.
        let dll: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
        unsafe {
            let lib = LoadLibraryW(dll.as_ptr());
            if lib.is_null() {
                return Vec::new();
            }
            let out = read_devices(lib);
            FreeLibrary(lib);
            out
        }
    }

    unsafe fn read_devices(lib: HMODULE) -> Vec<Device> {
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
                    // The engine requires Vulkan 1.3 and rejects a CPU device.
                    if props.api_version < VK_API_VERSION_1_3 || props.device_type == 4 {
                        continue;
                    }
                    let end = props
                        .device_name
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(props.device_name.len());
                    let name = String::from_utf8_lossy(&props.device_name[..end])
                        .trim()
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let mut mem: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
                    memory(dev, &mut mem);
                    let vram = mem.memory_heaps[..mem.memory_heap_count.min(16) as usize]
                        .iter()
                        .filter(|h| h.flags & VK_MEMORY_HEAP_DEVICE_LOCAL_BIT != 0)
                        .map(|h| h.size)
                        .max()
                        .unwrap_or(0);
                    out.push(Device {
                        name,
                        kind: kind(props.device_type),
                        vram_mb: vram / (1024 * 1024),
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
    use super::Device;

    pub fn devices() -> Vec<Device> {
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
}
