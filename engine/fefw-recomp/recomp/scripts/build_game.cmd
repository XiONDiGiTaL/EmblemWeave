@echo off
rem Configure and build the module libraries of one game (games\<id>), sccache kept in work\build\sccache.
rem   scripts\build_game.cmd <id> [extra ninja args, e.g. -j 20 | -k 0 | <id>_sdk]
rem A game folder games\<id> is a CMake project that takes NX_ROOT (the repository root: game\, work\) and has a target <id>_modules.
rem Environment: NX_JOBS (default 20) = compile processes; NX_BUILD (default <root>\work\build\<id>).
setlocal
if "%~1"=="" (
    echo usage: scripts\build_game.cmd ^<id^> [ninja args]
    exit /b 2
)
rem the game id without quotes, and the arguments after it (cmd's shift does not change %*)
set "GAME=%~1"
set "ARGS=%*"
call set "ARGS=%%ARGS:*%1=%%"
set "HERE=%~dp0"
set "ROOT=%HERE%..\.."
for %%I in ("%ROOT%") do set "ROOT=%%~fI"
if not exist "%HERE%..\games\%GAME%\CMakeLists.txt" (
    echo no game folder games\%GAME%
    exit /b 2
)
if not defined NX_BUILD set "NX_BUILD=%ROOT%\work\build\%GAME%"
if not defined NX_JOBS set "NX_JOBS=20"
if not defined SCCACHE_DIR set "SCCACHE_DIR=%ROOT%\work\build\sccache"
if not defined SCCACHE_SERVER_PORT set "SCCACHE_SERVER_PORT=24339"
if not defined SCCACHE_CACHE_SIZE set "SCCACHE_CACHE_SIZE=20G"
rem CMAKE_SUPPRESS_REGENERATION: the configure on the next line runs on every build, and Ninja must not
rem re-run it from a timestamp comparison. A release extracted with files newer than the clock, or an
rem output written in the same tick as an input, otherwise loops "Re-running CMake" until Ninja's
rem 100-try limit (issue #34).
rem CMAKE_NINJA_FORCE_RESPONSE_FILE: Ninja otherwise writes object paths into the command line, and
rem cmd.exe splits an unquoted `&` in an absolute source path (for example `PC Ports & Recomps`) into
rem two commands (issue #38). Response files keep every path one argument, including spaces,
rem parentheses, Unicode and long object lists.
call "%HERE%nx_env.cmd" cmake -S "%HERE%..\games\%GAME%" -B "%NX_BUILD%" -G Ninja -DCMAKE_TOOLCHAIN_FILE="%HERE%..\cmake\clang-cl.cmake" -DCMAKE_BUILD_TYPE=Release -DNX_ROOT="%ROOT%" -DCMAKE_SUPPRESS_REGENERATION=ON -DCMAKE_NINJA_FORCE_RESPONSE_FILE=ON
if errorlevel 1 exit /b 1
call "%HERE%nx_env.cmd" cmake --build "%NX_BUILD%" --target %GAME%_modules -- -j %NX_JOBS%%ARGS%
exit /b %errorlevel%
