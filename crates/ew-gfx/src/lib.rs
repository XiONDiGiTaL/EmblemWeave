//! The host graphics device behind EmblemWeave's NVN layer.
//!
//! NVN objects the game creates are backed by real Vulkan objects on the host.
//! This crate owns the Vulkan instance, the physical device choice and the
//! logical device with its graphics queue. It is created once per process, on
//! the first `nvnDeviceInitialize`, and every NVN host service reaches it
//! through [`device`].
//!
//! The loader is opened at run time (`ash`'s `loaded` feature), so a machine
//! without Vulkan gets a clear error from `nvnDeviceInitialize` instead of an
//! executable that fails to start.

use ash::vk;
use std::ffi::CStr;
use std::sync::{Arc, Mutex, OnceLock};

/// What the game's NVN layer needs to know about the host device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    /// Vulkan API version the device supports, as `major.minor.patch`.
    pub api: (u32, u32, u32),
    pub driver: String,
    pub queue_family: u32,
    pub max_texture_2d: u32,
    pub min_uniform_alignment: u64,
    pub min_storage_alignment: u64,
}

pub struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    pub physical: vk::PhysicalDevice,
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub info: DeviceInfo,
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn version(v: u32) -> (u32, u32, u32) {
    (vk::api_version_major(v), vk::api_version_minor(v), vk::api_version_patch(v))
}

impl Gpu {
    /// Create the instance and the logical device. Prefers a discrete GPU,
    /// then an integrated one, and requires Vulkan 1.3 and a graphics queue.
    pub fn new() -> Result<Gpu, String> {
        let entry = unsafe { ash::Entry::load() }.map_err(|e| format!("the Vulkan loader could not be opened: {e}"))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"EmblemWeave")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"EmblemWeave NVN")
            .api_version(vk::API_VERSION_1_3);
        let create = vk::InstanceCreateInfo::default().application_info(&app);
        let instance =
            unsafe { entry.create_instance(&create, None) }.map_err(|e| format!("vkCreateInstance failed: {e}"))?;

        let candidates = unsafe { instance.enumerate_physical_devices() }.map_err(|e| e.to_string())?;
        let rank = |t: vk::PhysicalDeviceType| match t {
            vk::PhysicalDeviceType::DISCRETE_GPU => 0,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
            _ => 2,
        };
        let mut best: Option<(vk::PhysicalDevice, vk::PhysicalDeviceProperties, u32)> = None;
        for pd in candidates {
            let props = unsafe { instance.get_physical_device_properties(pd) };
            if props.api_version < vk::API_VERSION_1_3 {
                continue;
            }
            let families = unsafe { instance.get_physical_device_queue_family_properties(pd) };
            let Some(qf) = families.iter().position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)) else {
                continue;
            };
            let better = best.as_ref().is_none_or(|(_, b, _)| rank(props.device_type) < rank(b.device_type));
            if better {
                best = Some((pd, props, qf as u32));
            }
        }
        let Some((physical, props, queue_family)) = best else {
            unsafe { instance.destroy_instance(None) };
            return Err("no Vulkan 1.3 device with a graphics queue was found".into());
        };

        let priorities = [1.0f32];
        let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(queue_family).queue_priorities(&priorities)];
        let mut v13 = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true).synchronization2(true);
        let create = vk::DeviceCreateInfo::default().queue_create_infos(&queues).push_next(&mut v13);
        let device = match unsafe { instance.create_device(physical, &create, None) } {
            Ok(d) => d,
            Err(e) => {
                unsafe { instance.destroy_instance(None) };
                return Err(format!("vkCreateDevice failed: {e}"));
            }
        };
        let queue = unsafe { device.get_device_queue(queue_family, 0) };

        let mut driver_props = vk::PhysicalDeviceDriverProperties::default();
        let mut props2 = vk::PhysicalDeviceProperties2::default().push_next(&mut driver_props);
        unsafe { instance.get_physical_device_properties2(physical, &mut props2) };
        let driver = driver_props.driver_name_as_c_str().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
        let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy().into_owned();
        let info = DeviceInfo {
            name,
            vendor_id: props.vendor_id,
            device_id: props.device_id,
            api: version(props.api_version),
            driver,
            queue_family,
            max_texture_2d: props.limits.max_image_dimension2_d,
            min_uniform_alignment: props.limits.min_uniform_buffer_offset_alignment,
            min_storage_alignment: props.limits.min_storage_buffer_offset_alignment,
        };
        Ok(Gpu { _entry: entry, instance, physical, device, queue, info })
    }
}

static GPU: OnceLock<Mutex<Option<Arc<Gpu>>>> = OnceLock::new();

/// The process's host device, created on first use. Creation failures are
/// returned each time rather than cached, so a driver installed later is found.
pub fn device() -> Result<Arc<Gpu>, String> {
    let slot = GPU.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(g) = guard.as_ref() {
        return Ok(g.clone());
    }
    let g = Arc::new(Gpu::new()?);
    *guard = Some(g.clone());
    Ok(g)
}

#[cfg(test)]
mod tests {
    #[test]
    fn creates_a_device_when_vulkan_is_present() {
        // Machines without a Vulkan driver report an error instead of a device;
        // either outcome is valid, a panic is not.
        match super::device() {
            Ok(g) => {
                assert!(!g.info.name.is_empty());
                assert!(g.info.api.0 >= 1);
                assert!(g.info.max_texture_2d >= 4096);
            }
            Err(e) => assert!(!e.is_empty()),
        }
    }
}
