@echo off
rem Builds and runs one test program (tests/lift/progs/NN_name.cpp) natively with the host C++ runtime and prints the non-zero results.
rem   tests\tools\native_check.cmd tests\lift\progs\20_eh_hier.cpp [out.expected]
rem Exit code 0 = built and ran; the results[] array is written to out.expected (default: %TEMP%\nx_native.expected).
setlocal
set "HERE=%~dp0"
call "%HERE%..\..\scripts\env.cmd"
set "SRC=%~f1"
set "OUT=%~2"
if "%OUT%"=="" set "OUT=%TEMP%\nx_native.expected"
set "EXE=%TEMP%\nx_native_check.exe"
clang-cl /nologo /EHsc /std:c++20 /O1 -Wno-everything /I "%HERE%..\lift\native" /I "%HERE%..\..\..\recomp\oracle\e2e\progs" "%SRC%" "%HERE%..\lift\native\native_main.cpp" /Fe"%EXE%" /Fo"%TEMP%\\" || exit /b 1
"%EXE%" "%OUT%" || exit /b 2
echo native run ok: %OUT%
python -I "%HERE%dump_results.py" "%OUT%"
exit /b 0
