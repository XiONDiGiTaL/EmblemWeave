//! NVN textures, texture pools and sampler pools on the host.
//!
//! A texture's storage is a range of a memory pool. Titles ship texture data
//! already laid out for the console GPU (block-linear tiling) and copy it into
//! pool memory themselves, so the storage size the host reports has to be the
//! block-linear size the data was built for. [`storage_size`] computes it with
//! the GPU's published tiling rules: 64-byte by 8-row GOBs stacked into blocks
//! whose height shrinks with the image. The host renderer reads the texels from
//! guest memory with the same rules.
//!
//! Formats are identified by their NVN enumeration value. The numbering is
//! confirmed by the game's own table from BNTX surface formats (a public file
//! format) to NVN formats at rodata 0x1b6c004: every BNTX format there maps to
//! the value listed below. Only formats whose value and texel size are
//! established are listed; any other stops the run naming the value.

use crate::hle::{read_bytes, HostResult, HostServices};
use crate::nvn::called;
use ew_lift::State;
use std::collections::BTreeMap;

const NVN_TRUE: u64 = 1;

/// Texel block of a format: block width and height in texels, bytes per block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub w: u32,
    pub h: u32,
    pub bytes: u32,
}

const fn px(bytes: u32) -> Option<Block> {
    Some(Block { w: 1, h: 1, bytes })
}

const fn bc(bytes: u32) -> Option<Block> {
    Some(Block { w: 4, h: 4, bytes })
}

/// NVN format value to its texel block.
pub fn format_block(format: u32) -> Option<Block> {
    match format {
        0x01..=0x04 => px(1),             // R8, R8SN, R8UI, R8I
        0x05..=0x09 => px(2),             // R16F, R16, R16SN, R16UI, R16I
        0x0A..=0x0C => px(4),             // R32F, R32UI, R32I
        0x0D..=0x10 => px(2),             // RG8 family
        0x11..=0x15 => px(4),             // RG16 family
        0x16..=0x18 => px(8),             // RG32 family
        0x19..=0x1C => px(3),             // RGB8 family
        0x1D..=0x21 => px(6),             // RGB16 family
        0x22..=0x24 => px(12),            // RGB32 family
        0x25..=0x28 => px(4),             // RGBA8 family
        0x29..=0x2D => px(8),             // RGBA16 family
        0x2E..=0x30 => px(16),            // RGBA32 family
        0x31 => px(1),                    // STENCIL8
        0x32 => px(2),                    // DEPTH16
        0x33 | 0x34 | 0x35 => px(4),      // DEPTH24, DEPTH32F, DEPTH24_STENCIL8
        0x36 => px(8),                    // DEPTH32F_STENCIL8
        0x37 | 0x38 => px(4),             // RGBX8_SRGB, RGBA8_SRGB
        0x39..=0x3C => px(2),             // RGBA4, RGB5, RGB5A1, RGB565
        0x3D..=0x40 => px(4),             // RGB10A2, RGB10A2UI, R11G11B10F, RGB9E5F
        0x41 | 0x42 | 0x45 | 0x46 => bc(8), // DXT1 (BC1), linear and sRGB
        0x43 | 0x44 | 0x47 | 0x48 => bc(16), // DXT3, DXT5 (BC2, BC3)
        0x49 | 0x4A => bc(8),             // RGTC1 (BC4)
        0x4B | 0x4C => bc(16),            // RGTC2 (BC5)
        0x4D..=0x50 => bc(16),            // BPTC (BC7, BC6H)
        // ASTC, linear 0x79..=0x86 and sRGB 0x87..=0x94, 16 bytes per block.
        0x79..=0x94 => {
            const DIMS: [(u32, u32); 14] = [
                (4, 4), (5, 4), (5, 5), (6, 5), (6, 6), (8, 5), (8, 6),
                (8, 8), (10, 5), (10, 6), (10, 8), (10, 10), (12, 10), (12, 12),
            ];
            let (w, h) = DIMS[((format - 0x79) % 14) as usize];
            Some(Block { w, h, bytes: 16 })
        }
        0x95 | 0x97 | 0x98 => px(2),      // BGR565, BGR5A1, A1BGR5
        0x9A | 0x9C => px(4),             // BGRA8, BGRA8_SRGB
        _ => None,
    }
}

/// NVN texture targets.
pub const TARGET_3D: u32 = 2;

/// NVN texture flags this layer acts on.
pub const FLAG_LINEAR: u32 = 0x10;

#[derive(Debug, Clone, Default)]
pub struct TexDesc {
    pub target: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub levels: u32,
    pub format: u32,
    pub samples: u32,
    pub flags: u32,
    pub stride: u32,
    pub swizzle: [u32; 4],
    pub depth_stencil_mode: u32,
    pub pool: u64,
    pub offset: u64,
}

impl TexDesc {
    fn defaults() -> TexDesc {
        TexDesc { target: 1, width: 1, height: 1, depth: 1, levels: 1, swizzle: [2, 3, 4, 5], ..TexDesc::default() }
    }
}

fn gob_height(height_blocks: u32, mut bh: u32) -> u32 {
    while bh > 1 && height_blocks <= (bh / 2) * 8 {
        bh /= 2;
    }
    bh
}

fn gob_depth(depth: u32, mut bd: u32) -> u32 {
    while bd > 1 && depth <= bd / 2 {
        bd /= 2;
    }
    bd
}

/// One mip level of a texture's storage.
#[derive(Debug, Clone)]
pub struct Level {
    /// Byte offset of the level within a layer.
    pub offset: u64,
    /// Bytes per row of blocks, padded to whole GOBs (block-linear) or the
    /// stride (linear).
    pub row_bytes: u64,
    /// Rows of blocks, padded to whole blocks of GOBs.
    pub rows: u64,
    /// Depth slices, and the block height and depth in GOBs.
    pub depth: u64,
    pub gob_h: u64,
    pub gob_d: u64,
}

/// Where every texel block of a texture lives in its storage.
#[derive(Debug, Clone)]
pub struct Layout {
    pub block: Block,
    pub levels: Vec<Level>,
    pub layer_stride: u64,
    pub layers: u64,
    pub linear: bool,
}

impl Layout {
    pub fn size(&self) -> u64 {
        self.layer_stride * self.layers
    }

    /// Byte offset of block (bx, by) in slice `bz` of `level` and `layer`.
    pub fn offset(&self, level: usize, layer: u64, bx: u64, by: u64, bz: u64) -> u64 {
        let lv = &self.levels[level];
        let base = layer * self.layer_stride + lv.offset;
        let xb = bx * self.block.bytes as u64;
        if self.linear {
            return base + bz * lv.row_bytes * lv.rows + by * lv.row_bytes + xb;
        }
        let gob_block = 512 * lv.gob_h * lv.gob_d;
        let gobs_wide = lv.row_bytes / 64;
        let blocks_high = lv.rows / (8 * lv.gob_h);
        base + (bz / lv.gob_d) * gobs_wide * blocks_high * gob_block
            + (by / (8 * lv.gob_h)) * gobs_wide * gob_block
            + (xb / 64) * gob_block
            + (bz % lv.gob_d) * 512 * lv.gob_h
            + ((by / 8) % lv.gob_h) * 512
            + gob_offset(xb % 64, by % 8)
    }
}

/// Offset of byte column `x` (0..64) of row `y` (0..8) inside one GOB.
pub fn gob_offset(x: u64, y: u64) -> u64 {
    (x / 32) * 256 + (y / 2) * 64 + ((x % 32) / 16) * 32 + (y % 2) * 16 + (x % 16)
}

/// The storage layout of a texture: linear rows when it is linear, otherwise
/// the GOB tiling, with each array layer starting on a block boundary.
pub fn layout(d: &TexDesc) -> Result<Layout, String> {
    let b = format_block(d.format).ok_or_else(|| format!("texture format 0x{:x} is not established", d.format))?;
    let blocks = |v: u32, n: u32| v.max(1).div_ceil(n) as u64;
    let samples = d.samples.max(1) as u64;
    let (depth, layers) = if d.target == TARGET_3D { (d.depth.max(1), 1) } else { (1, d.depth.max(1)) };
    if d.flags & FLAG_LINEAR != 0 {
        let row = if d.stride != 0 { d.stride as u64 } else { (blocks(d.width, b.w) * b.bytes as u64).div_ceil(32) * 32 };
        let rows = blocks(d.height, b.h);
        let level = Level { offset: 0, row_bytes: row * samples, rows, depth: 1, gob_h: 1, gob_d: 1 };
        return Ok(Layout { block: b, levels: vec![level], layer_stride: row * samples * rows, layers: layers as u64, linear: true });
    }
    let h0 = blocks(d.height, b.h) as u32;
    let bh0 = gob_height(h0, 16);
    let bd0 = if d.target == TARGET_3D { gob_depth(depth, 32) } else { 1 };
    let mut levels = Vec::new();
    let mut layer = 0u64;
    for l in 0..d.levels.max(1) {
        let (w, h, z) = ((d.width >> l).max(1), (d.height >> l).max(1), (depth >> l).max(1));
        let (wb, hb) = (blocks(w, b.w), blocks(h, b.h));
        let bh = gob_height(hb as u32, bh0) as u64;
        let bd = gob_depth(z, bd0) as u64;
        let row_bytes = (wb * b.bytes as u64 * samples).div_ceil(64) * 64;
        let rows = hb.div_ceil(8 * bh) * 8 * bh;
        let slices = (z as u64).div_ceil(bd) * bd;
        levels.push(Level { offset: layer, row_bytes, rows, depth: z as u64, gob_h: bh, gob_d: bd });
        layer += row_bytes * rows * slices;
    }
    let block_bytes = 512 * bh0 as u64 * bd0 as u64;
    let stride = if layers > 1 { layer.div_ceil(block_bytes) * block_bytes } else { layer };
    Ok(Layout { block: b, levels, layer_stride: stride, layers: layers as u64, linear: false })
}

/// Bytes a texture occupies in its pool.
pub fn storage_size(d: &TexDesc) -> Result<u64, String> {
    layout(d).map(|l| l.size())
}

#[derive(Default)]
pub struct Textures {
    /// Builder state, by builder address.
    pub builders: BTreeMap<u64, TexDesc>,
    /// Initialized textures.
    pub textures: BTreeMap<u64, TexDesc>,
    /// Texture and sampler pools: object to (memory pool, offset, descriptors).
    pub texture_pools: BTreeMap<u64, (u64, u64, u64)>,
    pub sampler_pools: BTreeMap<u64, (u64, u64, u64)>,
    /// Descriptor slots: (pool, id) to the texture or sampler registered there.
    pub texture_slots: BTreeMap<(u64, u64), (u64, u64)>,
    pub sampler_slots: BTreeMap<(u64, u64), u64>,
}

fn tex<R>(state: &State, f: impl FnOnce(&mut Textures) -> R) -> R {
    state.host.ext.with::<Textures, R>(f)
}

fn w32(v: u64) -> u32 {
    (v & 0xFFFF_FFFF) as u32
}

/// Every `nvnTextureBuilderSet*` that shapes the texture.
fn builder_set(state: &mut State) -> HostResult {
    let name = called(state);
    let (b, a1, a2, a3, a4) = (state.x[0], state.x[1], state.x[2], state.x[3], state.x[4]);
    let field = name.trim_start_matches("nvnTextureBuilderSet").to_string();
    let ok = tex(state, |t| {
        if field == "Defaults" {
            t.builders.insert(b, TexDesc::defaults());
            return true;
        }
        let d = t.builders.entry(b).or_insert_with(TexDesc::defaults);
        match field.as_str() {
            "Device" | "GLTextureName" | "PackagedTextureData" | "PackagedTextureLayout" => {}
            "Flags" => d.flags = w32(a1),
            "Target" => d.target = w32(a1),
            "Width" => d.width = w32(a1),
            "Height" => d.height = w32(a1),
            "Depth" => d.depth = w32(a1),
            "Size1D" => d.width = w32(a1),
            "Size2D" => (d.width, d.height) = (w32(a1), w32(a2)),
            "Size3D" => (d.width, d.height, d.depth) = (w32(a1), w32(a2), w32(a3)),
            "Levels" => d.levels = w32(a1),
            "Format" => d.format = w32(a1),
            "Samples" => d.samples = w32(a1),
            "Swizzle" => d.swizzle = [w32(a1), w32(a2), w32(a3), w32(a4)],
            "DepthStencilMode" => d.depth_stencil_mode = w32(a1),
            "Stride" => d.stride = w32(a1),
            "Storage" => (d.pool, d.offset) = (a1, a2),
            _ => return false,
        }
        true
    });
    if ok {
        HostResult::Returned(0)
    } else {
        HostResult::Unavailable(format!("{name} is not modelled"))
    }
}

fn builder_desc(state: &State, b: u64) -> TexDesc {
    tex(state, |t| t.builders.get(&b).cloned().unwrap_or_else(TexDesc::defaults))
}

fn builder_get_storage_size(state: &mut State) -> HostResult {
    match storage_size(&builder_desc(state, state.x[0])) {
        Ok(n) => HostResult::Returned(n),
        Err(e) => HostResult::Unavailable(format!("nvnTextureBuilderGetStorageSize: {e}")),
    }
}

/// Alignment of a texture's storage: one GOB, the unit of the tiling.
fn builder_get_storage_alignment(_state: &mut State) -> HostResult {
    HostResult::Returned(512)
}

fn desc_field(d: &TexDesc, name: &str) -> Option<u64> {
    Some(match name {
        "Flags" => d.flags as u64,
        "Target" => d.target as u64,
        "Width" => d.width as u64,
        "Height" => d.height as u64,
        "Depth" => d.depth as u64,
        "Levels" => d.levels as u64,
        "Format" => d.format as u64,
        "Samples" => d.samples as u64,
        "DepthStencilMode" => d.depth_stencil_mode as u64,
        "Stride" => d.stride as u64,
        "MemoryPool" => d.pool,
        "MemoryOffset" => d.offset,
        _ => return None,
    })
}

fn builder_get(state: &mut State) -> HostResult {
    let name = called(state);
    let d = builder_desc(state, state.x[0]);
    match desc_field(&d, name.trim_start_matches("nvnTextureBuilderGet")) {
        Some(v) => HostResult::Returned(v),
        None => HostResult::Unavailable(format!("{name} is not modelled")),
    }
}

/// `nvnTextureInitialize(NVNtexture*, const NVNtextureBuilder*)`.
fn texture_initialize(state: &mut State) -> HostResult {
    let (t, b) = (state.x[0], state.x[1]);
    let d = builder_desc(state, b);
    if let Err(e) = storage_size(&d) {
        return HostResult::Unavailable(format!("nvnTextureInitialize: {e}"));
    }
    tex(state, |x| x.textures.insert(t, d));
    HostResult::Returned(NVN_TRUE)
}

fn texture_finalize(state: &mut State) -> HostResult {
    let t = state.x[0];
    tex(state, |x| x.textures.remove(&t));
    HostResult::Returned(0)
}

fn texture_get(state: &mut State) -> HostResult {
    let name = called(state);
    let t = state.x[0];
    let Some(d) = tex(state, |x| x.textures.get(&t).cloned()) else {
        return HostResult::Unavailable(format!("{name} on texture 0x{t:x}, which was never initialized"));
    };
    let field = name.trim_start_matches("nvnTextureGet");
    let v = match field {
        "StorageSize" => storage_size(&d).ok(),
        "TextureAddress" => state.host.ext.with::<crate::nvn_mem::Memory, _>(|m| m.pools.get(&d.pool).map(|p| p.gpu + d.offset)),
        _ => desc_field(&d, field),
    };
    match v {
        Some(v) => HostResult::Returned(v),
        None => HostResult::Unavailable(format!("{name} is not modelled")),
    }
}

/// `nvnTexturePoolInitialize(pool, memoryPool, offset, numDescriptors)` and the
/// sampler pool equivalent.
fn texture_pool_initialize(state: &mut State) -> HostResult {
    let (p, mp, off, n) = (state.x[0], state.x[1], state.x[2], state.x[3] & 0xFFFF_FFFF);
    tex(state, |x| x.texture_pools.insert(p, (mp, off, n)));
    HostResult::Returned(NVN_TRUE)
}

fn sampler_pool_initialize(state: &mut State) -> HostResult {
    let (p, mp, off, n) = (state.x[0], state.x[1], state.x[2], state.x[3] & 0xFFFF_FFFF);
    tex(state, |x| x.sampler_pools.insert(p, (mp, off, n)));
    HostResult::Returned(NVN_TRUE)
}

fn pool_get(state: &mut State) -> HostResult {
    let name = called(state);
    let p = state.x[0];
    let sampler = name.starts_with("nvnSamplerPool");
    let entry = tex(state, |x| if sampler { x.sampler_pools.get(&p).copied() } else { x.texture_pools.get(&p).copied() });
    let Some((mp, off, n)) = entry else {
        return HostResult::Unavailable(format!("{name} on pool 0x{p:x}, which was never initialized"));
    };
    match name.rsplit("Get").next().unwrap_or("") {
        "MemoryPool" => HostResult::Returned(mp),
        "MemoryOffset" => HostResult::Returned(off),
        "Size" => HostResult::Returned(n),
        _ => HostResult::Unavailable(format!("{name} is not modelled")),
    }
}

fn pool_finalize(state: &mut State) -> HostResult {
    let p = state.x[0];
    tex(state, |x| {
        x.texture_pools.remove(&p);
        x.sampler_pools.remove(&p);
    });
    HostResult::Returned(0)
}

/// `nvnTexturePoolRegisterTexture(pool, id, texture, view)` and
/// `RegisterImage`: the slot now names that texture.
fn texture_pool_register(state: &mut State) -> HostResult {
    let (p, id, t, view) = (state.x[0], state.x[1] & 0xFFFF_FFFF, state.x[2], state.x[3]);
    tex(state, |x| x.texture_slots.insert((p, id), (t, view)));
    HostResult::Returned(0)
}

/// `nvnSamplerPoolRegisterSampler(pool, id, sampler)` and the builder form.
fn sampler_pool_register(state: &mut State) -> HostResult {
    let (p, id, s) = (state.x[0], state.x[1] & 0xFFFF_FFFF, state.x[2]);
    tex(state, |x| x.sampler_slots.insert((p, id), s));
    HostResult::Returned(0)
}

/// `nvnDeviceGetTextureHandle(device, textureID, samplerID)`: the GPU's
/// combined handle, texture index in the low 20 bits and sampler index above.
fn device_get_texture_handle(state: &mut State) -> HostResult {
    let (t, s) = (state.x[1] & 0xF_FFFF, state.x[2] & 0xFFF);
    HostResult::Returned(t | (s << 20))
}

/// `nvnDeviceGetImageHandle(device, imageID)` and the texel-fetch form.
fn device_get_image_handle(state: &mut State) -> HostResult {
    HostResult::Returned(state.x[1] & 0xF_FFFF)
}

/// `nvnTextureViewGet*(view, out...)`: copies what the matching setter
/// recorded into the out pointers and returns true, or returns false when the
/// view never set it, which is how NVN reports an unset view field.
fn view_get(state: &mut State) -> HostResult {
    let name = called(state);
    let view = state.x[0];
    let setter = name.replacen("Get", "Set", 1);
    let outs = match name.trim_start_matches("nvnTextureViewGet") {
        "Levels" | "Layers" => 2,
        "Swizzle" => 4,
        "Format" | "DepthStencilMode" | "Target" => 1,
        _ => return HostResult::Unavailable(format!("{name} is not modelled")),
    };
    let args = state.host.ext.with::<crate::nvn::Nvn, _>(|n| n.state.get(&view).and_then(|m| m.get(&setter)).cloned());
    let Some(args) = args else { return HostResult::Returned(0) };
    for i in 0..outs {
        let out = state.x[1 + i];
        if out != 0 {
            if let Err(e) = crate::hle::write_bytes(state, out, &(args[i] as u32).to_le_bytes()) {
                return e;
            }
        }
    }
    HostResult::Returned(NVN_TRUE)
}

/// Guest address of a texture's storage.
fn storage_address(state: &State, d: &TexDesc) -> Option<u64> {
    state.host.ext.with::<crate::nvn_mem::Memory, _>(|m| m.pools.get(&d.pool).filter(|p| p.storage != 0).map(|p| p.storage + d.offset))
}

/// Copy texels between packed guest memory and a texture's tiled storage.
/// `region` is NVNcopyRegion: x, y, z offset then width, height, depth, in
/// texels; a view selects the base level and layer.
fn transfer_texels(state: &mut State, to_texture: bool, row_stride: Option<u64>, image_stride: Option<u64>) -> HostResult {
    let (t, view, region, data) = (state.x[0], state.x[1], state.x[2], state.x[3]);
    let Some(d) = tex(state, |x| x.textures.get(&t).cloned()) else {
        return HostResult::Unavailable(format!("texel transfer on texture 0x{t:x}, which was never initialized"));
    };
    let lay = match layout(&d) {
        Ok(l) => l,
        Err(e) => return HostResult::Unavailable(e),
    };
    let Some(base) = storage_address(state, &d) else {
        return HostResult::Unavailable(format!("texture 0x{t:x} has no CPU-visible storage"));
    };
    let r = match read_bytes(state, region, 24) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let field = |i: usize| i32::from_le_bytes(r[i * 4..i * 4 + 4].try_into().unwrap()).max(0) as u64;
    let (x, y, z, w, h, dd) = (field(0), field(1), field(2), field(3), field(4), field(5).max(1));
    let view_arg = |setter: &str, i: usize| {
        if view == 0 {
            return 0;
        }
        state.host.ext.with::<crate::nvn::Nvn, _>(|n| n.state.get(&view).and_then(|m| m.get(setter)).map(|a| a[i] & 0xFFFF_FFFF).unwrap_or(0))
    };
    let level = view_arg("nvnTextureViewSetLevels", 0) as usize;
    let first_layer = view_arg("nvnTextureViewSetLayers", 0);
    if level >= lay.levels.len() {
        return HostResult::Unavailable(format!("texel transfer on level {level} of a {}-level texture", lay.levels.len()));
    }
    let blk = lay.block;
    let (bx0, by0) = (x / blk.w as u64, y / blk.h as u64);
    let (bw, bh) = (w.div_ceil(blk.w as u64), h.div_ceil(blk.h as u64));
    let bytes = blk.bytes as u64;
    let row = row_stride.unwrap_or(bw * bytes);
    let image = image_stride.unwrap_or(row * bh);
    let is3d = d.target == TARGET_3D;
    let size = lay.size();
    for k in 0..dd {
        let (layer, slice) = if is3d { (0, z + k) } else { (first_layer + z + k, 0) };
        for j in 0..bh {
            for i in 0..bw {
                let off = lay.offset(level, layer, bx0 + i, by0 + j, slice);
                if off + bytes > size {
                    return HostResult::Unavailable(format!("texel transfer outside texture 0x{t:x}"));
                }
                let (tex_at, mem_at) = (base + off, data + k * image + j * row + i * bytes);
                let (from, to) = if to_texture { (mem_at, tex_at) } else { (tex_at, mem_at) };
                let (f, o) = (from as usize, to as usize);
                if f + bytes as usize > state.mem.len() || o + bytes as usize > state.mem.len() {
                    return HostResult::Unavailable(format!("texel transfer at 0x{from:x} or 0x{to:x} is outside guest memory"));
                }
                state.mem.copy_within(f..f + bytes as usize, o);
            }
        }
    }
    HostResult::Returned(0)
}

/// `nvnTextureWriteTexels(texture, view, region, data)`.
fn write_texels(state: &mut State) -> HostResult {
    transfer_texels(state, true, None, None)
}

/// `nvnTextureWriteTexelsStrided(texture, view, region, data, rowStride, imageStride)`.
fn write_texels_strided(state: &mut State) -> HostResult {
    let (rs, is) = (state.x[4], state.x[5]);
    transfer_texels(state, true, (rs != 0).then_some(rs), (is != 0).then_some(is))
}

/// `nvnTextureReadTexels(texture, view, region, data)`.
fn read_texels(state: &mut State) -> HostResult {
    transfer_texels(state, false, None, None)
}

fn read_texels_strided(state: &mut State) -> HostResult {
    let (rs, is) = (state.x[4], state.x[5]);
    transfer_texels(state, false, (rs != 0).then_some(rs), (is != 0).then_some(is))
}

/// Flushes and invalidates: the host reads guest memory in place.
fn coherent(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

pub fn register(s: &mut HostServices) {
    s.register_prefix("nvn:nvnTextureViewSet", crate::nvn::record_state, "texture view state on the host");
    s.register_prefix("nvn:nvnTextureViewGet", view_get, "texture view state on the host");
    s.register_prefix("nvn:nvnTextureBuilderSet", builder_set, "texture builder state on the host");
    s.register_prefix("nvn:nvnTextureBuilderGet", builder_get, "texture builder state on the host");
    s.register_prefix("nvn:nvnTextureGet", texture_get, "texture state on the host");
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("nvnTextureBuilderGetStorageSize", builder_get_storage_size, "block-linear size by the GPU's tiling rules"),
        ("nvnTextureBuilderGetStorageAlignment", builder_get_storage_alignment, "one GOB"),
        ("nvnTextureInitialize", texture_initialize, "a texture over pool storage, for formats the host knows"),
        ("nvnTextureFinalize", texture_finalize, "forgets the texture"),
        ("nvnTextureWriteTexels", write_texels, "tiled into the texture's block-linear storage"),
        ("nvnTextureWriteTexelsStrided", write_texels_strided, "tiled into the texture's block-linear storage"),
        ("nvnTextureReadTexels", read_texels, "read back out of the block-linear storage"),
        ("nvnTextureReadTexelsStrided", read_texels_strided, "read back out of the block-linear storage"),
        ("nvnTextureFlushTexels", coherent, "guest memory is read in place, so it is coherent"),
        ("nvnTextureInvalidateTexels", coherent, "guest memory is read in place, so it is coherent"),
        ("nvnTexturePoolInitialize", texture_pool_initialize, "descriptor slots kept on the host"),
        ("nvnSamplerPoolInitialize", sampler_pool_initialize, "descriptor slots kept on the host"),
        ("nvnTexturePoolGetMemoryPool", pool_get, "the pool the descriptors live in"),
        ("nvnTexturePoolGetMemoryOffset", pool_get, "the descriptors' offset"),
        ("nvnTexturePoolGetSize", pool_get, "the descriptor count"),
        ("nvnSamplerPoolGetMemoryPool", pool_get, "the pool the descriptors live in"),
        ("nvnSamplerPoolGetMemoryOffset", pool_get, "the descriptors' offset"),
        ("nvnSamplerPoolGetSize", pool_get, "the descriptor count"),
        ("nvnTexturePoolFinalize", pool_finalize, "forgets the pool"),
        ("nvnSamplerPoolFinalize", pool_finalize, "forgets the pool"),
        ("nvnTexturePoolRegisterTexture", texture_pool_register, "slot to texture, kept on the host"),
        ("nvnTexturePoolRegisterImage", texture_pool_register, "slot to image, kept on the host"),
        ("nvnSamplerPoolRegisterSampler", sampler_pool_register, "slot to sampler, kept on the host"),
        ("nvnSamplerPoolRegisterSamplerBuilder", sampler_pool_register, "slot to sampler state, kept on the host"),
        ("nvnDeviceGetTextureHandle", device_get_texture_handle, "texture index low 20 bits, sampler index above"),
        ("nvnDeviceGetSeparateTextureHandle", device_get_image_handle, "texture index"),
        ("nvnDeviceGetSeparateSamplerHandle", device_get_image_handle, "sampler index"),
        ("nvnDeviceGetImageHandle", device_get_image_handle, "image index"),
        ("nvnDeviceGetTexelFetchHandle", device_get_image_handle, "texture index"),
    ];
    for (name, handler, claim) in table {
        s.register(&format!("nvn:{name}"), *handler, claim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d2(w: u32, h: u32, format: u32) -> TexDesc {
        TexDesc { width: w, height: h, format, ..TexDesc::defaults() }
    }

    #[test]
    fn block_linear_sizes_follow_the_gob_rules() {
        // 3840x2160 RGB10A2: 15360-byte rows, 2160 rows padded to 16-GOB blocks (128 rows).
        assert_eq!(storage_size(&d2(3840, 2160, 0x3D)).unwrap(), 15360 * 2176);
        // A small texture shrinks its block height: 16x16 RGBA8 is 64 bytes by 16 rows.
        assert_eq!(storage_size(&d2(16, 16, 0x25)).unwrap(), 64 * 16);
        // BC1 counts 4x4 blocks: 256x256 is 64x64 blocks of 8 bytes.
        assert_eq!(storage_size(&d2(256, 256, 0x41)).unwrap(), 512 * 64);
    }

    #[test]
    fn mips_and_layers_add_up() {
        let mut d = d2(64, 64, 0x25);
        d.levels = 2;
        // 256 bytes x 64 rows, then 128 -> 128 bytes... padded to 64-byte rows: 128 x 32 rows.
        assert_eq!(storage_size(&d).unwrap(), 256 * 64 + 128 * 32);
        d.levels = 1;
        d.target = 4;
        d.depth = 3;
        let one = storage_size(&d2(64, 64, 0x25)).unwrap();
        assert!(storage_size(&d).unwrap() >= 3 * one);
    }

    #[test]
    fn astc_blocks_follow_the_bntx_table() {
        // BNTX 0x3406 (ASTC 8x8 sRGB) maps to NVN 0x8e in the game's table.
        assert_eq!(format_block(0x8e), Some(Block { w: 8, h: 8, bytes: 16 }));
        assert_eq!(format_block(0x79), Some(Block { w: 4, h: 4, bytes: 16 }));
        assert_eq!(format_block(0x86), Some(Block { w: 12, h: 12, bytes: 16 }));
        assert_eq!(format_block(0x94), Some(Block { w: 12, h: 12, bytes: 16 }));
    }

    #[test]
    fn tiling_matches_the_gob_layout() {
        // Within one GOB: 16-byte runs, two rows per 64 bytes, halves of 256.
        assert_eq!(gob_offset(0, 0), 0);
        assert_eq!(gob_offset(16, 0), 32);
        assert_eq!(gob_offset(0, 1), 16);
        assert_eq!(gob_offset(0, 2), 64);
        assert_eq!(gob_offset(32, 0), 256);
        // 256x256 RGBA8: 1024-byte rows, 16-GOB blocks. Block (16, 0) is
        // byte 64 of row 0, the second GOB column.
        let l = layout(&d2(256, 256, 0x25)).unwrap();
        assert_eq!(l.offset(0, 0, 16, 0, 0), 512 * 16);
        // Row 8 is the second GOB down the same block column.
        assert_eq!(l.offset(0, 0, 0, 8, 0), 512);
        // Row 128 starts the second row of blocks: 16 GOB columns of 8 KiB.
        assert_eq!(l.offset(0, 0, 0, 128, 0), 16 * 512 * 16);
        // Every block of the level lands at a distinct offset inside it.
        let mut seen = std::collections::HashSet::new();
        for by in 0..256 {
            for bx in 0..256 {
                let o = l.offset(0, 0, bx, by, 0);
                assert!(o + 4 <= l.size());
                assert!(seen.insert(o));
            }
        }
    }

    #[test]
    fn unknown_formats_are_refused() {
        assert!(storage_size(&d2(4, 4, 0x200)).is_err());
    }
}
