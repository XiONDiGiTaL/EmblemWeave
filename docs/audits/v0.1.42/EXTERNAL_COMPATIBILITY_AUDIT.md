# External Compatibility Audit

## Freebuff Integration
- **Repository**: https://recomplauncher.com/game/emblemweave
- **Merge Commit**: 20396fc (pending validation)
- **Outstanding Branches**: Audit required

## GitHub Issues
- **Repository**: https://github.com/XiONDiGiTaL/EmblemWeave/issues
- **Open Issues (12)**: #79, #78, #77, #76, #75, #70, #59, #41, #37, #29, #26, #7
- **Fixed in v0.1.42**: #75 (infrastructure), #76 (infrastructure), #77 (FIXED), #78 (infrastructure)
- **New Issue #79**: Save files not being read (reported Nov 22, 2026) - needs investigation

## Dependencies

### Runtime Dependencies
| Component | Version | Source |
|-----------|---------|--------|
| SDL3 | 3.4.18 | Bundled |
| Vulkan | 1.4.359 | System (AMD driver 2.0.416) |
| FFmpeg | N-123619-g44ad73031d-20260325 | E:/EmblemWeave/upscalers/ffmpeg |
| dav1d | Latest | Bundled (with x86-64 asm) |
| libvpx | 1.16.0 | Bundled |
| zstd | Latest | Bundled |
| FSR3 | Latest | amd_fidelityfx_loader_vk.dll |

### FFmpeg DLLs (Dynamic Loading)
- avcodec-62.dll
- avutil-60.dll
- swresample-6.dll

### System Requirements
- Windows 10/11 (64-bit)
- Vulkan 1.3+ capable GPU
- AVX2 support (for dav1d assembly)
- 16 GB+ RAM recommended

## Configuration Compatibility
- **Portable Mode**: Enabled via portable.txt at launcher directory
- **Data Root**: E:\EmblemWeave\data
- **Engine Source**: C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\engine\fefw-recomp
- **Overlay**: work/romfs-overlay (21,468 dirs, 252,693 files)

## Known Compatibility Issues
1. **RomFS Mismatch**: 252695 files vs 252686 expected - may cause missing file errors
2. **FSR3 Markers**: vkCmdWriteBufferMarkerAMD not available - frame gen falls back
3. **No fefw.sites**: Frame-fix hooks not validated against build
4. **Issue #79**: Save files not being read - may be config or path issue

## Validation Checklist
- [ ] Freebuff merge commit 20396fc validated
- [ ] All GitHub issues reviewed and cross-referenced
- [ ] Dependency versions documented
- [ ] System requirements verified
- [ ] Portable mode tested
- [ ] Overlay extraction verified
- [ ] Issue #79 investigated