# Code-mod interfaces (NxModApi version 6)

EmblemWeave's code-mod table (`include/nxrt/mod_api.h`) is at version 6. Version 6 appends one member,
`get_interface`, and changes nothing else: every earlier member keeps its position, type and meaning, so a mod
compiled against version 3 or 4 keeps working without recompilation.

```c
typedef const void* (*NxModGetInterfaceFn)(const struct NxModApi* api, const char* name, uint32_t version);
```

`get_interface` is at the end of `NxModApi`, after `panel_visible`. A mod checks `api->version >= 6` and
`api->size >= offsetof(NxModApi, get_interface) + sizeof(api->get_interface)` before reading it, as for every
versioned member.

## Lookup rules

- **Name**: ASCII, case-sensitive, canonical `<owner>.<local-name>` with at least two dot-separated segments.
  Segments use `[a-z0-9-]` and start with a letter or digit. Upper-case, empty segments, a leading or trailing dot,
  and names over 127 bytes are invalid.
- **Version**: the caller passes the interface version it was written for. `0` asks for any compatible version.
  An interface table is append-only, so a table at version N also answers a request for an older version. A request
  for a version newer than the table the host provides returns `NULL`.
- **Existence probe**: `get_interface(api, name, 0) != NULL`. There is no separate `has()` member.
- **Invalid, unknown and incompatible names all return `NULL`**; a mod must not treat a miss as fatal.
- **Tables start with `uint32_t size` and `uint32_t version`** and grow only at the end. Check both before reading a
  member added after the version the caller was written for.
- **Thread and phase**: the lookup may be called from `nxmod_early`, `nxmod_init`, a hook or a frame callback, on any
  thread. A returned table stays valid for the run.

## Names and ownership

A name belongs to the project that defines it. A host answers for its own prefix and for the shared names it
implements; it does not synthesize another host's names.

| Name | Owner | Meaning |
| --- | --- | --- |
| `nxp.nxmod` | NXP (shared) | Version 1. The host's own `NxModApi` table. |
| `nxp.ui` | NXP (shared) | The portable panel canvas (Trinity's adapter). |
| `nxp.settings`, `nxp.files`, `nxp.frame` | NXP (shared) | The other NXP v1 services (Trinity's adapter). |
| `emblemweave.*` | EmblemWeave | Host-only interfaces and game integrations. |
| `fefw-recomp.*` | fefw-recomp | Host-only interfaces and game integrations. |
| `obeliskchamber.*` | ObeliskChamber | Host-only interfaces, e.g. `obeliskchamber.list` version 1. |

`nxp.nxmod` is the bridge to the legacy table and is registered by EmblemWeave itself. It answers the `api` pointer
the caller was given: the table itself, not a copy. That table keeps its existing `version`-then-`size` order and
its own version field; it is the one bridged table, not a new interface structure. A plugin built on a shared layer
gets hooks, `call_guest`, guest memory and file edits through it on every host that has version 6.

`get_interface(api, "nxp.nxmod", 1) == api` is the expected result. Version 2 of that name does not exist.

## Registration

Registration lives in `include/nxrt/mod_interfaces.h` and `src/services/mod_interfaces.cpp`.

```cpp
nxrt::nxmod::InterfaceRegistry& r = nxrt::nxmod::interfaces();
r.add("emblemweave.ui.panel", 1, &panel_table);          // a fixed table
r.add_resolver("nxp.nxmod", 1, resolve_native_api);     // a table that depends on the caller
```

Rules enforced by the registry:

- the name must be canonical (see above) and the version must be at least 1;
- the table (or resolver) must not be null;
- a name is registered once. A second registration is refused, including the same table at another version:
  replacing a live table would give two mods different answers for one name. A duplicate is a host bug, not a
  negotiation;
- a resolver receives the `NxModApi` passed to `get_interface`; the registry lock is not held while it runs, and its
  result must stay valid for the run.

A host adds its interfaces before it loads mods. `nxp.nxmod` version 1 is registered on first use.

## Version 5 is not part of EmblemWeave's lineage

EmblemWeave goes from 4 to 6. Version 5 was ObeliskChamber's `open_list` variant, which placed `open_list` in the
`add_panel` slot. EmblemWeave never shipped that meaning: its version-4 tail stays `add_panel` / `panel_visible` at
version 6. A mod must not infer `open_list` from `api->version >= 5`; that is exactly the fragile check version 6
replaces. Panels stay on `add_panel` (or `nxp.ui` through Trinity's adapter) and a host-specific list screen is a
named interface such as `obeliskchamber.list`.

## Tests

`tests/test_mod_api.cpp` (group `mod_api`) covers:

- compile-time offsets for every version-3/4 member against the pre-v6 table, and `get_interface` as the last member;
- registration, lookup, version 0, older/newer requests, unknown and invalid names, duplicates and bad versions;
- the `nxp.nxmod` bridge through the real `get_interface`, including the names this host does not own;
- `tests/mods/test_mod.cpp`, a fixture DLL compiled as a version-3, version-4 and version-6 mod, loaded by
  `LoadLibraryExW` and called with the table `make_mod_api()` builds for `load_dll_mods`.

Run it with `build\release\Release\nxrt_tests.exe mod_api` (or `nxrt_tests mod_api` in a single-config build).
