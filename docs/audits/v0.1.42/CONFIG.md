# EmblemWeave Configuration Reference

## fefw.ini (E:\EmblemWeave\data\play\fefw.ini)

### [log]
- `path = fefw.log` - Log file path

### [fs]
- `exefs = E:/EmblemWeave/engine/fefw-recomp/game/dump/exefs` - ExeFS dump path
- `romfs = H:/Downloads/qBittorent Downloads/Fire Emblem꞉ Fortune's Weave + Update 1.1.1/Fire Emblem꞉ Fortune's Weave + Update 1.1.1.nup` - RomFS .nup path
- `romfs_overlay = C:/Users/xiond/.config/freebuff-desktop/local-projects/413b2383-4be0-4b3c-b36e-5f485402a327/EmblemWeave/engine/fefw-recomp/work/romfs-overlay` - Overlay extraction path
- `save_dir = E:/EmblemWeave/data/save` - Save data directory
- `mods = E:/EmblemWeave/data/play/mods` - Mods directory

### [nvn] - NVN Graphics Backend
- `replay_threads = 1` - Replay thread count
- `tex_prep_threads = 0` - Texture prep threads (0 = auto)
- `tex_prep_mb = 1024` - Texture prep memory budget (MB)
- `tex_prep = 1` - Enable texture prep
- `staging_ring_mb = 256` - Staging ring buffer (MB)
- `prewarm = 1` - Prewarm pipelines
- `pool_upload_ms = 6` - Pool upload time budget (ms)
- `lean_draws = 1` - Optimize draw calls
- `gpl = 1` - Graphics pipeline library
- `compile_priority = below` - Shader compile priority
- `compile_eco = 1` - Eco compile mode
- `shader_cache = E:/EmblemWeave/engine/fefw-recomp/work/shaders/spirv` - SPIR-V cache
- `bc7_cache = E:/EmblemWeave/engine/fefw-recomp/work/bc7_cache` - BC7 cache
- `render_prep = 0` - Render preparation
- `render_prep_threads = 0` - Render prep threads
- `pool_prefetch = true` - Enable background pool prefetch

### [upscale] - Upscaling
- `sharpness = 0.8` - FSR sharpness
- `scene_scale = 100` - Scene scale %
- `quality = quality` - Quality preset
- `fsr_dll = E:/EmblemWeave/data/play/upscalers/fsr/amd_fidelityfx_loader_vk.dll` - FSR DLL
- `mode = fsr4` - FSR mode

### [dlss] - DLSS/FSR3 Frame Gen
- `frame_gen = fsr3` - Frame generation
- `upscaler = fsr3` - Upscaler

### [diag] - Diagnostics
- `watchdog_seconds = 120` - Watchdog timeout (seconds)

### [fefw] - Game Settings
- `output = 1080p` - Output resolution
- `fps60 = 1` - Enable 60 FPS
- `fps120 = 1` - Enable 120 FPS
- `render_ratio = 1.0` - Render resolution scale
- `shadow_size = 8192` - Shadow map size
- `quality = docked` - Quality preset
- `packed_nup = 1` - Use packed .nup

### [gfx] - Graphics
- `residency_percent = 80` - Memory residency target %
- `residency = auto` - Residency mode
- `present_guard = auto` - Present guard
- `gpu = AMD Radeon RX 7900 XT` - GPU override

### [media] - Media Playback
- `movie_hwaccel = 1` - Hardware acceleration
- `movie_decoder = ffmpeg` - Decoder backend
- `ffmpeg_dir = E:/EmblemWeave/upscalers/ffmpeg` - FFmpeg directory

### [perf] - Performance
- `target_fps = 179` - Target FPS
- `stability = 1` - Stability mode
- `scene_profiles = 1` - Scene profiles
- `scene_override = auto` - Scene override
- `mode = automatic` - Performance mode
- `max_mip_bias = 0.75` - Max mip bias
- `enabled = 1` - Enable performance features
- `adaptive_mip = 0` - Adaptive mip

### [engine] - Engine Metadata
- `built_by = 0.1.42` - Builder version
- `packed_nup = 1` - Packed .nup flag

## Launcher Configuration
- **Portable Mode**: Enabled via `portable.txt` in launcher directory
- **Data Root**: `E:\EmblemWeave\data`
- **Engine Source**: `C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\engine\fefw-recomp`
- **Deployed Runtime**: `E:\EmblemWeave\fefw.exe`
- **Launcher Version**: 0.1.42

## Runtime Launch Command
```
fefw.exe --ini data/play/fefw.ini
```

## Save Data Structure
```
E:/EmblemWeave/data/save/
└── Profile1/
    └── slot_000/
        └── (save files)
```

## Mods Structure
```
E:/EmblemWeave/data/play/mods/
└── Weapon Durability/
    └── romfs/
        └── (mod files)
```