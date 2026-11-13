//! The window the game's frames appear in.
//!
//! NVN hands the host the texture it just rendered a frame into; this module
//! owns the Windows window, its Vulkan surface and swap chain, and puts those
//! pixels on screen. A frame arrives as ready-to-show BGRA8 rows (the NVN layer
//! untiles and converts the game's swap texture), is copied into a host-visible
//! staging buffer, copied into the next swap-chain image and presented.
//!
//! The presenter owns its own instance and device rather than sharing
//! [`crate::device`]: the NVN device is created before there is a window and
//! without the surface extensions, and keeping the two apart means a driver that
//! refuses a surface still runs the game headless.
//!
//! Nothing here draws the game. The pixels come from the NVN command buffers, so
//! until the renderer executes the game's draws a frame is whatever the game last
//! left in its swap texture.

use ash::vk;

/// Swap-chain images the presenter asks for.
const IMAGES: u32 = 3;

/// One frame's worth of BGRA8 pixels, top row first.
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub bgra: &'a [u8],
}

/// Everything the presenter owns, created in [`Presenter::open`] and torn down
/// on drop.
pub struct Presenter {
    window: platform::Window,
    entry: ash::Entry,
    instance: ash::Instance,
    surface_fn: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    swapchain_fn: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    format: vk::Format,
    extent: vk::Extent2D,
    /// Whether this surface wants RGBA rather than the frames' BGRA.
    swap_channels: bool,
    command_pool: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    image_available: vk::Semaphore,
    render_done: vk::Semaphore,
    memory: vk::DeviceMemory,
    staging: vk::Buffer,
    staging_size: u64,
    mapped: *mut u8,
    /// Frames presented so far.
    pub frames: u64,
}

impl Presenter {
    /// Open a window at the given client size and everything needed to show
    /// frames in it.
    pub fn open(title: &str, width: u32, height: u32) -> Result<Presenter, String> {
        let window = platform::Window::open(title, width, height)?;
        unsafe { Self::build(window) }
    }

    unsafe fn build(window: platform::Window) -> Result<Presenter, String> {
        let entry = ash::Entry::load().map_err(|e| format!("the Vulkan loader could not be opened: {e}"))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"EmblemWeave")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"EmblemWeave NVN")
            .api_version(vk::API_VERSION_1_3);
        let extensions = [vk::KHR_SURFACE_NAME.as_ptr(), vk::KHR_WIN32_SURFACE_NAME.as_ptr()];
        let create = vk::InstanceCreateInfo::default().application_info(&app).enabled_extension_names(&extensions);
        let instance = entry.create_instance(&create, None).map_err(|e| format!("the window's Vulkan instance failed: {e}"))?;

        let win32 = ash::khr::win32_surface::Instance::new(&entry, &instance);
        let info = vk::Win32SurfaceCreateInfoKHR::default().hinstance(window.instance as isize).hwnd(window.hwnd as isize);
        let surface = win32.create_win32_surface(&info, None).map_err(|e| format!("the game's window has no Vulkan surface: {e}"))?;
        let surface_fn = ash::khr::surface::Instance::new(&entry, &instance);

        // A device that can present to this window and has a graphics queue.
        let mut chosen = None;
        for pd in instance.enumerate_physical_devices().map_err(|e| e.to_string())? {
            if instance.get_physical_device_properties(pd).api_version < vk::API_VERSION_1_3 {
                continue;
            }
            let families = instance.get_physical_device_queue_family_properties(pd);
            for (i, f) in families.iter().enumerate() {
                if !f.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                    continue;
                }
                if surface_fn.get_physical_device_surface_support(pd, i as u32, surface).unwrap_or(false) {
                    chosen = Some((pd, i as u32));
                    break;
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        let (physical, queue_family) = chosen.ok_or("no Vulkan device can present to the game's window")?;

        let priorities = [1.0f32];
        let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(queue_family).queue_priorities(&priorities)];
        let names = [vk::KHR_SWAPCHAIN_NAME.as_ptr()];
        let device_create = vk::DeviceCreateInfo::default().queue_create_infos(&queues).enabled_extension_names(&names);
        let device = instance.create_device(physical, &device_create, None).map_err(|e| format!("the window's Vulkan device failed: {e}"))?;
        let queue = device.get_device_queue(queue_family, 0);
        let swapchain_fn = ash::khr::swapchain::Device::new(&instance, &device);

        let pool = vk::CommandPoolCreateInfo::default().queue_family_index(queue_family).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = device.create_command_pool(&pool, None).map_err(|e| e.to_string())?;
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command = device.allocate_command_buffers(&alloc).map_err(|e| e.to_string())?[0];
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None)
            .map_err(|e| e.to_string())?;
        let sem = vk::SemaphoreCreateInfo::default();
        let image_available = device.create_semaphore(&sem, None).map_err(|e| e.to_string())?;
        let render_done = device.create_semaphore(&sem, None).map_err(|e| e.to_string())?;

        let caps = surface_fn
            .get_physical_device_surface_capabilities(physical, surface)
            .map_err(|e| format!("the window's surface reports no capabilities: {e}"))?;
        let (format, swap_channels) = pick_format(&surface_fn, physical, surface)?;
        let extent = swap_extent(&caps, window.client_size());

        let mut p = Presenter {
            window,
            entry,
            instance,
            surface_fn,
            surface,
            physical,
            device,
            queue,
            swapchain_fn,
            swapchain: vk::SwapchainKHR::null(),
            images: Vec::new(),
            views: Vec::new(),
            format,
            extent,
            swap_channels,
            command_pool,
            command,
            fence,
            image_available,
            render_done,
            memory: vk::DeviceMemory::null(),
            staging: vk::Buffer::null(),
            staging_size: 0,
            mapped: std::ptr::null_mut(),
            frames: 0,
        };
        p.create_swapchain(None)?;
        let bytes = p.extent.width as u64 * p.extent.height as u64 * 4;
        p.create_staging(bytes)?;
        Ok(p)
    }

    unsafe fn create_swapchain(&mut self, old: Option<vk::SwapchainKHR>) -> Result<(), String> {
        let caps = self
            .surface_fn
            .get_physical_device_surface_capabilities(self.physical, self.surface)
            .map_err(|e| format!("the window's surface reports no capabilities: {e}"))?;
        let modes = self
            .surface_fn
            .get_physical_device_surface_present_modes(self.physical, self.surface)
            .map_err(|e| format!("the window's surface reports no present modes: {e}"))?;
        self.extent = swap_extent(&caps, self.window.client_size());
        let present = if modes.contains(&vk::PresentModeKHR::MAILBOX) { vk::PresentModeKHR::MAILBOX } else { vk::PresentModeKHR::FIFO };
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(caps.min_image_count.saturating_add(1).max(IMAGES).min(caps.max_image_count.max(1)))
            .image_format(self.format)
            .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .image_extent(self.extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present)
            .clipped(true)
            .old_swapchain(old.unwrap_or(vk::SwapchainKHR::null()));
        self.swapchain = self
            .swapchain_fn
            .create_swapchain(&info, None)
            .map_err(|e| format!("the window's swap chain failed: {e}"))?;
        self.images = self.swapchain_fn.get_swapchain_images(self.swapchain).map_err(|e| e.to_string())?;
        for v in self.views.drain(..) {
            self.device.destroy_image_view(v, None);
        }
        for image in &self.images {
            let view = vk::ImageViewCreateInfo::default()
                .image(*image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(self.format)
                .subresource_range(vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).level_count(1).layer_count(1));
            self.views.push(self.device.create_image_view(&view, None).map_err(|e| e.to_string())?);
        }
        Ok(())
    }

    unsafe fn create_staging(&mut self, size: u64) -> Result<(), String> {
        if size == self.staging_size && self.staging != vk::Buffer::null() {
            return Ok(());
        }
        if self.staging != vk::Buffer::null() {
            self.device.unmap_memory(self.memory);
            self.device.destroy_buffer(self.staging, None);
            self.device.free_memory(self.memory, None);
            self.staging = vk::Buffer::null();
            self.mapped = std::ptr::null_mut();
        }
        let buffer = vk::BufferCreateInfo::default().size(size).usage(vk::BufferUsageFlags::TRANSFER_SRC).sharing_mode(vk::SharingMode::EXCLUSIVE);
        self.staging = self.device.create_buffer(&buffer, None).map_err(|e| format!("the frame buffer could not be created: {e}"))?;
        let needs = self.device.get_buffer_memory_requirements(self.staging);
        let props = self.instance.get_physical_device_memory_properties(self.physical);
        let kind = (0..props.memory_type_count)
            .find(|i| {
                needs.memory_type_bits & (1 << i) != 0
                    && props.memory_types[*i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT)
            })
            .ok_or("no host-visible memory for the frame buffer")?;
        let alloc = vk::MemoryAllocateInfo::default().allocation_size(needs.size).memory_type_index(kind);
        self.memory = self.device.allocate_memory(&alloc, None).map_err(|e| e.to_string())?;
        self.device.bind_buffer_memory(self.staging, self.memory, 0).map_err(|e| e.to_string())?;
        self.mapped = self
            .device
            .map_memory(self.memory, 0, needs.size, vk::MemoryMapFlags::empty())
            .map_err(|e| e.to_string())? as *mut u8;
        self.staging_size = size;
        Ok(())
    }

    /// Show one frame. Returns false once the window has been closed.
    pub fn show(&mut self, frame: Frame) -> Result<bool, String> {
        self.window.pump();
        if !self.is_open() {
            return Ok(false);
        }
        let given = frame.width as usize * frame.height as usize * 4;
        if frame.bgra.len() < given {
            return Err(format!(
                "a {}x{} frame needs {given} bytes, but {} were given",
                frame.width,
                frame.height,
                frame.bgra.len()
            ));
        }
        // The swap chain belongs to the window, so a frame of another size is
        // scaled to it: the game's swap texture is 4K whatever the window is.
        let scaled;
        let pixels: &[u8] = if frame.width == self.extent.width && frame.height == self.extent.height {
            &frame.bgra[..given]
        } else {
            scaled = scale_nearest(&frame.bgra[..given], frame.width, frame.height, self.extent.width, self.extent.height);
            &scaled
        };
        let bytes = pixels.len() as u64;
        unsafe {
            self.create_staging(bytes)?;
            // Put this frame's pixels into the mapped buffer the GPU will read.
            let dst = std::slice::from_raw_parts_mut(self.mapped, bytes as usize);
            if self.swap_channels {
                for (d, s) in dst.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = s[3];
                }
            } else {
                dst.copy_from_slice(pixels);
            }
            self.present_mapped()?;
        }
        Ok(true)
    }

    unsafe fn present_mapped(&mut self) -> Result<(), String> {
        self.device.wait_for_fences(&[self.fence], true, u64::MAX).map_err(|e| e.to_string())?;
        self.device.reset_fences(&[self.fence]).map_err(|e| e.to_string())?;
        let index = match self.swapchain_fn.acquire_next_image(self.swapchain, u64::MAX, self.image_available, vk::Fence::null()) {
            Ok((i, _)) => i,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.recreate()?;
                self.swapchain_fn
                    .acquire_next_image(self.swapchain, u64::MAX, self.image_available, vk::Fence::null())
                    .map_err(|e| e.to_string())?
                    .0
            }
            Err(e) => return Err(format!("the window could not hand over a frame: {e}")),
        };

        self.device.reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty()).map_err(|e| e.to_string())?;
        let begin = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        self.device.begin_command_buffer(self.command, &begin).map_err(|e| e.to_string())?;
        let range = vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).level_count(1).layer_count(1);
        let image = self.images[index as usize];
        let barrier = |old: vk::ImageLayout, new: vk::ImageLayout, src: vk::AccessFlags, dst: vk::AccessFlags| {
            vk::ImageMemoryBarrier::default()
                .old_layout(old)
                .new_layout(new)
                .image(image)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .src_access_mask(src)
                .dst_access_mask(dst)
                .subresource_range(range)
        };
        // A presented image goes back to UNDEFINED when it is acquired again,
        // so the frame starts from a fresh layout every time.
        let to_transfer = barrier(vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE);
        self.device.cmd_pipeline_barrier(
            self.command,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).layer_count(1))
            .image_extent(vk::Extent3D { width: self.extent.width, height: self.extent.height, depth: 1 });
        self.device
            .cmd_copy_buffer_to_image(self.command, self.staging, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);
        let to_present = barrier(
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::PRESENT_SRC_KHR,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::empty(),
        );
        self.device.cmd_pipeline_barrier(
            self.command,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        );
        self.device.end_command_buffer(self.command).map_err(|e| e.to_string())?;

        let wait = [self.image_available];
        let stages = [vk::PipelineStageFlags::TRANSFER];
        let signal = [self.render_done];
        let commands = [self.command];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&wait)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&commands)
            .signal_semaphores(&signal);
        self.device.queue_submit(self.queue, &[submit], self.fence).map_err(|e| e.to_string())?;
        self.device.wait_for_fences(&[self.fence], true, u64::MAX).map_err(|e| e.to_string())?;

        let chains = [self.swapchain];
        let indices = [index];
        let present = vk::PresentInfoKHR::default().wait_semaphores(&signal).swapchains(&chains).image_indices(&indices);
        match self.swapchain_fn.queue_present(self.queue, &present) {
            Ok(_) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {}
            Err(e) => return Err(format!("the frame could not be presented: {e}")),
        }
        self.frames += 1;
        Ok(())
    }

    /// Rebuild the swap chain after the window changed size or the driver
    /// retired it.
    unsafe fn recreate(&mut self) -> Result<(), String> {
        self.device.device_wait_idle().map_err(|e| e.to_string())?;
        let old = self.swapchain;
        self.swapchain = vk::SwapchainKHR::null();
        self.create_swapchain(Some(old))?;
        if old != vk::SwapchainKHR::null() {
            self.swapchain_fn.destroy_swapchain(old, None);
        }
        let bytes = self.extent.width as u64 * self.extent.height as u64 * 4;
        self.create_staging(bytes)?;
        Ok(())
    }

    /// Whether the window is still open.
    pub fn is_open(&self) -> bool {
        !self.window.closed()
    }
}

/// The surface format to present in: BGRA8 when the surface offers it, since
/// the frames arrive as BGRA, otherwise whatever RGBA form it prefers.
unsafe fn pick_format(
    surface_fn: &ash::khr::surface::Instance,
    physical: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Result<(vk::Format, bool), String> {
    let formats = surface_fn
        .get_physical_device_surface_formats(physical, surface)
        .map_err(|e| format!("the window's surface reports no formats: {e}"))?;
    let wanted = [
        (vk::Format::B8G8R8A8_SRGB, false),
        (vk::Format::B8G8R8A8_UNORM, false),
        (vk::Format::R8G8B8A8_SRGB, true),
        (vk::Format::R8G8B8A8_UNORM, true),
    ];
    for (format, swap) in wanted {
        if formats.iter().any(|f| f.format == format && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR) {
            return Ok((format, swap));
        }
    }
    let first = formats.first().ok_or("the window's surface offers no image format")?;
    if first.format == vk::Format::UNDEFINED {
        return Ok((vk::Format::B8G8R8A8_SRGB, false));
    }
    Ok((first.format, false))
}

/// Scale BGRA8 rows to another size, nearest texel: the game's frame is shown
/// at the window's size, and a first frame does not need filtering.
#[allow(dead_code)]
fn scale_nearest(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let sy = (y * sh / dh.max(1)).min(sh.saturating_sub(1));
        for x in 0..dw {
            let sx = (x * sw / dw.max(1)).min(sw.saturating_sub(1));
            let s = (sy * sw + sx) * 4;
            let d = (y * dw + x) * 4;
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}

/// The swap-chain size for the window's current client area.
fn swap_extent(caps: &vk::SurfaceCapabilitiesKHR, client: (u32, u32)) -> vk::Extent2D {
    if caps.current_extent.width != u32::MAX {
        return caps.current_extent;
    }
    vk::Extent2D {
        width: client.0.clamp(caps.min_image_extent.width, caps.max_image_extent.width).max(1),
        height: client.1.clamp(caps.min_image_extent.height, caps.max_image_extent.height).max(1),
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            if !self.mapped.is_null() {
                self.device.unmap_memory(self.memory);
            }
            if self.staging != vk::Buffer::null() {
                self.device.destroy_buffer(self.staging, None);
                self.device.free_memory(self.memory, None);
            }
            for v in self.views.drain(..) {
                self.device.destroy_image_view(v, None);
            }
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_fn.destroy_swapchain(self.swapchain, None);
            }
            self.device.destroy_semaphore(self.image_available, None);
            self.device.destroy_semaphore(self.render_done, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.surface_fn.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
            let _ = &self.entry;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_keeps_every_texel_of_the_frame() {
        // 2x2 to 4x4: every source texel becomes a 2x2 block of the same colour.
        let src = [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
        let out = scale_nearest(&src, 2, 2, 4, 4);
        assert_eq!(out.len(), 64);
        for (at, want) in [(0, [1, 2, 3, 255]), (4, [1, 2, 3, 255]), (16, [1, 2, 3, 255]), (24, [4, 5, 6, 255]), (48, [7, 8, 9, 255]), (56, [10, 11, 12, 255])] {
            assert_eq!(&out[at..at + 4], &want, "at {at}");
        }
    }

    #[test]
    fn scaling_down_picks_source_texels() {
        let src: Vec<u8> = (0..4 * 4 * 4).map(|i| i as u8).collect();
        let out = scale_nearest(&src, 4, 4, 2, 2);
        assert_eq!(out.len(), 16);
        assert_eq!(&out[0..4], &src[0..4]);
        assert_eq!(&out[4..8], &src[8..12]);
        assert_eq!(&out[8..12], &src[32..36]);
        assert_eq!(&out[12..16], &src[40..44]);
    }
}

/// The Windows window itself: a plain top-level window with a message pump.
mod platform {
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, LoadCursorW, PeekMessageW,
        PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
        IDC_ARROW, MSG, PM_REMOVE, SW_SHOW, WM_CLOSE, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    /// The window's handles, kept as the pointer values Vulkan's surface
    /// creation wants. Whether the user closed the window is recorded in
    /// [`CLOSED`], which the window procedure can reach without a pointer.
    pub struct Window {
        pub instance: isize,
        pub hwnd: isize,
    }

    static CLOSED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                CLOSED.store(true, Ordering::SeqCst);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }

    impl Window {
        /// Create and show a window whose client area is about `width`x`height`.
        pub fn open(title: &str, width: u32, height: u32) -> Result<Window, String> {
            let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            let class_name: Vec<u16> = "EmblemWeaveFrame\0".encode_utf16().collect();
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wndproc),
                    hInstance: instance,
                    lpszClassName: class_name.as_ptr(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    ..std::mem::zeroed()
                };
                if RegisterClassW(&class) == 0 {
                    return Err(format!("the window class could not be registered: {}", std::io::Error::last_os_error()));
                }
                let hwnd = CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    wide.as_ptr(),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    width as i32 + 16,
                    height as i32 + 39,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    return Err(format!("the game's window could not be created: {}", std::io::Error::last_os_error()));
                }
                ShowWindow(hwnd, SW_SHOW);
                Ok(Window { instance: instance as isize, hwnd: hwnd as isize })
            }
        }

        /// Handle the messages that have arrived, so the window stays alive.
        pub fn pump(&mut self) {
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        /// Whether the user closed the window.
        pub fn closed(&self) -> bool {
            CLOSED.load(Ordering::SeqCst)
        }

        /// The window's client area, in pixels.
        pub fn client_size(&self) -> (u32, u32) {
            unsafe {
                let mut r: RECT = std::mem::zeroed();
                if GetClientRect(self.hwnd as HWND, &mut r) == 0 {
                    return (1280, 720);
                }
                ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32)
            }
        }
    }
}
