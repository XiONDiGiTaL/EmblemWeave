@echo off
rem Standalone build + run of a few testtools sources with the nxrt test framework, without the full runtime build (for the pure parser/format units).
rem   scripts\tt_unit.cmd <group> <source.cpp> [<source.cpp> ...]       e.g.  scripts\tt_unit.cmd tt_script src\testtools\tt_script.cpp tests\test_tt_script.cpp
rem Builds into build\unit\<group>.exe and runs it with the group name. Run from the checkout root (the script cd's there itself).
setlocal enabledelayedexpansion
cd /d "%~dp0.."
call "%~dp0env.cmd"
set GROUP=%1
shift
set SRCS=
:loop
if "%~1"=="" goto done
set SRCS=!SRCS! "%~1"
shift
goto loop
:done
if not exist build\unit\%GROUP% mkdir build\unit\%GROUP%
clang-cl /nologo /std:c++20 /EHsc /O1 /W3 -Wno-unused-parameter -Wno-missing-field-initializers -DNOMINMAX -DWIN32_LEAN_AND_MEAN -D_CRT_SECURE_NO_WARNINGS -I include -I tests -I "%NXRT_RECOMP_INCLUDE%" !SRCS! tests\test_main.cpp /Fe:build\unit\%GROUP%.exe /Fo:build\unit\%GROUP%\ || exit /b 1
build\unit\%GROUP%.exe %GROUP%
exit /b %errorlevel%
