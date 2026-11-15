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

rem The developer environment, with its own named failures: no vcvars, no clang-cl or no C++ workload stops the build
rem here, in a second, rather than inside a ninja edge twenty minutes later.
call "%HERE%nx_env.cmd"
if errorlevel 1 exit /b %errorlevel%

rem The compiler is named by full path, so a clang-cl earlier on PATH cannot replace the one that was validated.
set "NX_CC=%NX_LLVM%\clang-cl.exe"

cmake -S "%HERE%..\games\%GAME%" -B "%NX_BUILD%" -G Ninja -DCMAKE_TOOLCHAIN_FILE="%HERE%..\cmake\clang-cl.cmake" -DCMAKE_BUILD_TYPE=Release -DNX_ROOT="%ROOT%" -DCMAKE_C_COMPILER="%NX_CC%" -DCMAKE_CXX_COMPILER="%NX_CC%"
if errorlevel 1 (
    echo [emblemweave] CMake could not configure "%NX_BUILD%". The error above names the cause; a cache made by another
    echo [emblemweave] compiler or an older CMake is cleared by deleting its CMakeCache.txt.
    exit /b 1
)

cmake --build "%NX_BUILD%" --target %GAME%_modules -- -j %NX_JOBS%%ARGS%
if errorlevel 1 (
    echo [emblemweave] compiling the recompiled code failed. The first "error:" line above names the file and the cause.
    exit /b 1
)
exit /b 0
