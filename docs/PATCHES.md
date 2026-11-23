# ExeFS code patches

EmblemWeave can install code patches for `main` as an ExeFS mod and apply them
before the recompiler lifts the game. This is **not** a runtime patch: the built
`fefw.exe` contains the changed code, so installing, editing, enabling,
disabling or removing a patch requires **Build the game** before **Play**.

> **Status:** the patch formats in this document have shipped since 0.1.37.
> Game version 1.1.1 has not been verified. Do not describe a patch as supported
> on 1.1.1 until a build containing it has been validated.

## What is supported

| File | Format | Accepted location |
| --- | --- | --- |
| `<build id>.ips` | IPS, including RLE records | a mod's `exefs/`, including a subfolder |
| `<build id>.ips32` | IPS32 | same |
| `any-name.pchtxt` | IPSwitch `.pchtxt` | same |

The file name of an `.ips` or `.ips32` must start with the 40-hex build ID of
`main`; a 32-hex prefix is also accepted. A generic `main.ips` is refused because
it cannot be tied to a build. A `.pchtxt` carries its build ID in its
`@nsobid-` header instead, so its file name can be descriptive. Do not name it
after `sdk`, `subsdk0`–`subsdk2` or `rtld`: the module name is checked before the
header, and those modules are refused. `main.pchtxt` is accepted, but a
descriptive name is clearer.

Only `main` is supported. `sdk`, `subsdk0`–`subsdk2` and `rtld` are refused by
name. The build pipeline writes only `main` uncompressed, and a patch made
against an uncompressed module cannot be applied safely to compressed storage.

### Finding the build ID

The build ID is the 40-hex value printed by `prepare_exefs.py` and by the build
log (`main: build id ...`). In the uncompressed NSO it is the first 20 bytes of
the 32-byte build-ID field at offset `0x40`. EmblemWeave compares it with the
`main` it is about to build, not with the patch author's game version.

A 32-hex declaration matches the first 32 digits of the current 40-hex ID. For
example, both of these match an NSO whose ID starts `0123456789ABCDEF01234567`:

```text
0123456789ABCDEF0123456789ABCDEF
0123456789ABCDEF0123456789ABCDEF01234567
```

Any other length is rejected rather than guessed.

## Installing

A patch-only mod is a folder containing `exefs/` with one or more supported
patch files:

```text
mods/
└── My Patch/
    └── exefs/
        └── 0123456789ABCDEF0123456789ABCDEF01234567.ips
```

Install it from **Mods** with **Install archive** (`.zip`, `.7z`, `.rar`) or
**Install folder**, exactly like a RomFS mod. A mod may contain both `romfs/`
and `exefs/`; the launcher keeps them together. A folder whose name starts with
`_` or `-` is switched off, so its patch is not active.

The launcher copies the module from your dump and uncompresses it into
`game/dump/exefs/main`, then applies the active records to that copy before the
lifter reads it. **Your game dump is only read; it is never modified.** The
audit and state are written into the run folder (`data/play` in a portable
install, `%APPDATA%\EmblemWeave\play` otherwise):

- `patch-audit.json`: the last build's files, hashes, ranges and status;
- `patch-state.json`: the active set the built executable corresponds to.

## IPS and IPS32

- The header must be `PATCH` (IPS) or `IPS32`; an `.ips` file whose content says
  `IPS32` is accepted as IPS32, and vice versa. The magic decides.
- Offsets are absolute offsets in the **uncompressed NSO**, including its
  `0x100`-byte header. A write below `0x100` is refused.
- IPS uses 3-byte offsets; IPS32 uses 4-byte offsets. The end marker is `EOF`
  (IPS) or `EEOF` (IPS32).
- A two-byte record length of zero starts an RLE record: a two-byte run length
  and one value byte. A zero-length run is refused.
- A record that runs past the end of `main` is refused. A record that runs into
  the file's EOF marker is truncated and refused.
- Final-size records are **not** supported. A patch that tries to change the
  NSO's size is refused rather than truncating or extending an executable.
- Records within one file must not overlap, even if their bytes are identical.
  Merge duplicate writes in the source patch.
- Patch files over 16 MiB are refused.

## IPSwitch `.pchtxt`

The parser reads UTF-8 text, handles CRLF and LF, and ignores blank lines and
full-line comments beginning with `//`, `;` or `#`.

```text
@nsobid-0123456789ABCDEF0123456789ABCDEF01234567
@flag print_values
@flag offset_shift 0x100
@enabled
// an enabled section
1000 DE AD BE EF
1004 "AB\x43"
@disabled
// this record is parsed but not applied
1008 00 01 02
@enabled
1010 12 34
@stop
```

Rules:

- `@nsobid-<id>` must be the first directive and must appear once. It accepts
  32 or 40 hex digits and must match the current `main`.
- `@flag offset_shift <hex>` adds a signed hexadecimal shift to every later
  record offset. `0x100` converts a main address that starts after the NSO
  header to an uncompressed-NSO offset. A negative shift is accepted, but an
  offset that becomes negative or below `0x100` is refused.
- `@flag print_values` is accepted for compatibility; the launcher does not
  print values. Other `@flag` names are refused.
- `@enabled` and `@disabled` select whether later records are applied. A file
  whose records are all disabled is a valid no-op, not a build failure.
- `@stop` ends the file. Records after it are not read.
- A patch line is `<hex offset> <byte> <byte> ...`. Bytes must be exactly two
  hex digits each; `0x` on the offset is optional. A quoted ASCII string is also
  accepted and supports `\n`, `\r`, `\t`, `\0`, `\\`, `\"` and `\xNN`. Non-ASCII
  characters are refused.
- `@flag` must come before the first record. `@enabled` / `@disabled` / `@stop`
  may appear between records.
- Unknown directives, a missing build header, a line with no bytes, a malformed
  string and an out-of-range offset are errors; the build refuses rather than
  skipping them.

The supported subset is intentionally strict. If a patch uses a directive that
is not listed here, extend the parser and its tests rather than ignoring the
line.

## Conflicts and order

Every active patch file is parsed before the build. Patches are ordered by mod
name (ASCII case-insensitive) and then by path inside the mod, so filesystem
enumeration order never decides which write wins.

- **Different bytes in the same range:** the build is refused. The Mods page
  names both files and the range.
- **Identical bytes in the same range:** this is a duplicate write, not a
  disagreement. It is reported as a warning and written once; the build can
  continue.
- **Overlap within one patch:** refused, even if the bytes agree.
- **Non-overlapping records:** all applied in order.

There is no “last patch wins” rule. Two mods that intentionally write the same
range must be reconciled by disabling one or editing the patch.

## Rebuild rules

`patch-state.json` records the active fingerprint after a successful build. The
fingerprint includes each active patch's mod name, relative path, content hash,
format, target module and declared build ID, plus `main`'s build ID. A RomFS-only
change does not change it.

**Play** refuses to start and asks for a rebuild when:

- a patch is added, edited, renamed, enabled or removed;
- the active set has patches but no successful build has recorded a state;
- the last recorded build used a different `main` build ID;
- the active set contains an invalid patch or a differing conflict;
- the built executable predates the last build (the existing build guard).

Removing the last patch is also a change: the built executable still contains
the old bytes, so a rebuild is required to take them out. A failed build leaves
the old state in place, so Play stays blocked until a build succeeds.

Starting `fefw.exe` directly bypasses this guard. It runs the executable as it
was last built; the launcher cannot change that from outside.

## Troubleshooting

| Message or symptom | Meaning | What to do |
| --- | --- | --- |
| “no supported patch file was found” | The `exefs/` folder has no `.ips`, `.ips32` or `.pchtxt`, or the file is not in an `exefs/` folder | Put the file directly under `mods/<mod>/exefs/`; `.bps`, `.xdelta` and other formats are not supported |
| “the file name does not identify main's build ID or a known module” | An IPS file is named `main`, `patch`, or something else that is not 32/40 hex | Rename it to the build ID from the build log, e.g. `<40 hex>.ips` |
| “an IPS file must be named after main's build ID” | Same as above, caught after the module check | Rename it; do not use `main.ips` |
| “the file name names build X, but this main is Y” | The patch is for a different game version or update | Use a patch for this `main`, or build the matching dump; do not rename to hide the mismatch |
| “the patch targets build X but this main is Y” | The `.pchtxt` header does not match | Same: use the matching patch or dump |
| “writes into the NSO header” | The offset is below `0x100` | If the patch uses main addresses, add `@flag offset_shift 0x100`; otherwise fix the offset |
| “is past the end of main” | Wrong version, wrong offset, or a truncated patch | Check the build ID and the patch's target; do not pad the file |
| “overlap inside one patch” | Two records in one file write the same byte range | Merge them into one record or remove the duplicate |
| “both write ... with different bytes” | Two active patches disagree | Disable one, or edit them into one consistent patch |
| “the file ends before its EOF marker” / “missing its two-byte length” / “zero run length” | Truncated or malformed IPS/RLE | Rebuild the patch with a correct IPS writer |
| “patches for sdk are not supported” | The patch targets a module other than `main` | Only `main` patches are supported |
| “the installed ExeFS patches cannot be applied, so the build stopped before lifting” | The build refused an invalid or conflicting set | Read the named file and range in the build log; fix the set and build again |
| “the active ExeFS patches have changed since the last build” | Play is correctly refusing a stale executable | Press **Build the game**, then **Play** |
| The mod is installed and the build succeeds, but the game looks unchanged | The patch may target code that is not reached, or the game version may differ | Check `patch-audit.json` for the file, hash and ranges; confirm the build ID; 1.1.1 is unverified |
| The game still has a removed patch | The executable was built with it and not rebuilt | Rebuild; removing a patch is a build change |

## Limits and non-goals

- No runtime patching. Code is recompiled ahead of time.
- No support for patching `sdk`, `subsdk0`–`subsdk2` or `rtld`.
- No final-size IPS extension, no `.bps`, `.xdelta` or arbitrary binary patching.
- No automatic discovery of patches outside a mod's `exefs/` folder.
- No claim that a patch works on 1.1.1 until that build is validated.
- The launcher never writes to the user's dump; it only reads it. The patched
  `main` is the launcher-owned copy under `game/dump/exefs/`.
