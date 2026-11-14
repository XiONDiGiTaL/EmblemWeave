# ka: known-answer tests of the lifter (no Unicorn)

Runs the A64 cases of ScarletNX's `tests/a64_ported_cases.h` through nxrecomp's lifter: every instruction a case executes is lifted to C++, compiled with clang-cl (`ka_host.cpp` plus the generated `ka_cases.cpp`), run on the host, and each of the case's expectations is checked.
Run: `tools/.venv/Scripts/python.exe oracle/ka/run_ka.py [--build DIR] [--cases FILE] [-v]`; the exit code is 0 only when no case fails.
A RUN stops at its first `b .` (the ported runner's end of execution). A case is skipped only for a real branch among the instructions that execute, a literal load of the code, or a window the host cannot map or that is not set.
PRESET_NCE cases move their window from address 0 to 0x7300000000 and rebase the addresses they hold. A failing FPSR expectation, including an X register read under the QC bit after an mrs of FPSR, counts as FPSR-not-modelled, a known gap, not as a failure.
Provenance: the cases come from ScarletNX `tests/a64_ported_cases.h` and its runner `a64_ported_run.inc` (GPL-3.0-or-later, ScarletNX contributors), which ported them from rEden's and dynarmic's A64 tests; the header is read from `refs/` and not copied here.
