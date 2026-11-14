# NVN struct sizes: what the game says

The practice mod's `nvn.h` (the header the first table was built from) is for an older NVN than the game's (API 55). A size from that header is only a starting guess: the game's own code is the authority, because an object it embeds next to its own fields cannot be larger than the room it leaves. Debug note 121 found the first mismatch (NVNtextureView). `docs/S4e.md` ("Struct sizes") has the audit; this file keeps the evidence.

Sizes below were read from `main.elf` with IDA (functions named as in the binary). "Header" is the practice mod's `nvn.h`. Where the game fixes the size, `scripts/gen_nvn_table.py` (`GAME_SIZES`) overrides the header and `nvn_sizes.inc` carries source mask 8.

| struct | header | game | evidence |
|---|---|---|---|
| NVNdevice | 0x3000 | 0x3000 | `NVNManaged::CDevice` keeps its init flag at +0x3000 (`CDevice::CDevice`, `Init`) |
| NVNqueue | 0x2000 | 0x2000 | `NVNManaged::CQueue` flag at +0x2000 |
| NVNwindow | 0x180 | 0x180 | `NVNManaged::CWindow` flag at +384 |
| NVNmemoryPool | 0x100 | 0x100 | `NVNManaged::CMemPool` first field after it at +256 |
| NVNbuffer | 0x30 | 0x30 | `NVNManaged::CBuffer` keeps its memory pointer at +48 |
| NVNtexture | 0xC0 | **0x80** | `NVNManaged::CTexture` keeps its memory pointer at +128, its flag at +144 |
| NVNprogram | 0xC0 | **0x80** | `NVNManaged::CProgram` flag at +128 |
| NVNsampler | 0x30 | **0x40** | `NVNManaged::CSampler` flag at +64 |
| NVNtextureView | 0x28 | **0x10** | `CGraphicsTextureToken_NVN`: view at +0x40, its own fields from +0x50 (debug note 121) |
| NVNsync | 0x40 | 0x40 | `CGraphicsFenceManager_NVN::CFenceBundle::InitializeFences` steps 64 bytes per sync |
| NVNcommandBuffer | 0xA0 | 0xA0 | `CGraphicsQueueCommandBuffer_NVN` keeps it at +32 and zeroes +32 to +192 |
| NVNtexturePool, NVNsamplerPool | 0x20 | 0x20 | `CGraphicsDescriptorTable_NVN`: texture pool +96, sampler pool +128, mutex +160 |
| NVNdepthStencilState, NVNcolorState, NVNpolygonState, NVNchannelMaskState, NVNmultisampleState | 8, 4, 4, 4, 0x18 | same | `CGraphicsStateCache_NVN::Init`: depth-stencil +704, colour +696, polygon +744, channel mask +748, multisample +752 (its own flags at +778) |
| NVNsamplerBuilder | 0x60 | at most 0x40 | the only stack variable of `sub_10DC50` (the sampler builder code) is 64 bytes |
| NVNtextureBuilder | 0x80 | 0x80 | stack variables of 128 bytes (`BuildForSparseTile`, `BuildFromContext`) |
| NVNdeviceBuilder, NVNqueueBuilder, NVNmemoryPoolBuilder, NVNbufferBuilder | 0x40 | at most 0x40 | 64-byte stack variables (`CDevice::Init`, `CQueue::Init`, `CMemPool::Init`, `sub_351970`) |

Stack variables only bound a builder from above (a compiler reuses a slot for several variables), so the real sizes of the other builders are not known from the game. The back end keeps the header's sizes for them (0x40, the event builder 0x20); no stack slot the game uses for a builder is smaller than that. The back end never writes outside the bytes its own layout needs, except where it clears a whole builder or state object in `SetDefaults` (0x40 at most).

Practice-mod header disagreements that are now settled by the game: texture 0xC0 -> 0x80, program 0xC0 -> 0x80, sampler 0x30 -> 0x40, texture view 0x28 -> 0x10, sampler builder 0x60 -> 0x40.

Not an NVN size but found on the way: `NVNtextureSparseTileLayout` is 16 bytes (four words; the game reads words 1 to 3 as the tile width, height and depth in texels). The header has a one-byte placeholder; the runtime header now declares the four words.
