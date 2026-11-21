@echo off
rem Configure and build the module libraries of one game (games\<id>), sccache kept in work\build\sccache.
rem   scripts\build_game.cmd <id> [extra ninja args, e.g. -j 20 | -k 0 | <id>_sdk]
rem   scripts\build_game.cmd --structured-events <build_id> <game_id> [toolchain_lock_path] (emit JSON Lines events)
rem A game folder games\<id> is a CMake project that takes NX_ROOT (the repository root: game\, work\) and has a target <id>_modules.
rem Environment: NX_JOBS (default: logical CPUs minus two, capped at 16) = compile processes;
rem NX_BUILD (default <root>\work\build\<id>). The EmblemWeave launcher always supplies a memory-aware NX_JOBS.
setlocal
if "%~1"=="" (
    echo usage: scripts\build_game.cmd ^<id^> [ninja args]
    exit /b 2
)

rem Check for structured events mode
if "%~1"=="--structured-events" (
    set "BUILD_ID=%~2"
    set "GAME=%~3"
    set "TOOLCHAIN_LOCK=%~4"
    if "%BUILD_ID%"=="" (
        echo usage: scripts\build_game.cmd --structured-events ^<build_id^> ^<game_id^> [toolchain_lock_path]
        exit /b 2
    )
    if "%GAME%"=="" (
        echo usage: scripts\build_game.cmd --structured-events ^<build_id^> ^<game_id^> [toolchain_lock_path]
        exit /b 2
    )

    set "HERE=%~dp0"
    set "ROOT=%HERE%..\.."
    for %%I in ("%ROOT%") do set "ROOT=%%~fI"

    if not defined NX_BUILD set "NX_BUILD=%ROOT%\work\build\%GAME%"
    if not defined NX_JOBS set /a NX_JOBS=%NUMBER_OF_PROCESSORS%-2
    if not defined NX_JOBS set "NX_JOBS=1"
    if %NX_JOBS% LSS 1 set "NX_JOBS=1"
    if %NX_JOBS% GTR 16 set "NX_JOBS=16"
    if not defined SCCACHE_DIR set "SCCACHE_DIR=%ROOT%\work\build\sccache"
    if not defined SCCACHE_SERVER_PORT set "SCCACHE_SERVER_PORT=24339"
    if not defined SCCACHE_CACHE_SIZE set "SCCACHE_CACHE_SIZE=20G"

    rem Emit structured events via PowerShell
    powershell -NoProfile -ExecutionPolicy Bypass -Command ^
        "$ErrorActionPreference='Stop'; ^
        Import-Module '%HERE%..\..\runtime\scripts\BuildEvents.psm1' -ArgumentList '%BUILD_ID%'; ^
        Initialize-EventWriter; ^
        $fingerprint = '%ROOT%|%GAME%|%BUILD_ID:~0,8%'; ^
        $config = @{jobs=%NX_JOBS%; memory_limit_mb=$null; sccache_enabled=$true; incremental=$true; target='%GAME%_modules'; features=@()}; ^
        Start-BuildEvent -ToolchainFingerprint $fingerprint -Config $config; ^
        Start-StageEvent -StageId 'configure' -StageName 'Configure (%GAME%)' -StageIndex 1 -TotalStages 2; ^
        Progress-StageEvent -StageId 'configure' -Progress 0.1 -Message 'Running CMake configure'; ^
        " ^
        2>&1

    call "%HERE%nx_env.cmd" cmake -S "%HERE%..\games\%GAME%" -B "%NX_BUILD%" -G Ninja -DCMAKE_TOOLCHAIN_FILE="%HERE%..\cmake\clang-cl.cmake" -DCMAKE_BUILD_TYPE=Release -DNX_ROOT="%ROOT%" -DCMAKE_SUPPRESS_REGENERATION=ON -DCMAKE_NINJA_FORCE_RESPONSE_FILE=ON
    if errorlevel 1 (
        powershell -NoProfile -ExecutionPolicy Bypass -Command ^
            "Import-Module '%HERE%..\..\runtime\scripts\BuildEvents.psm1' -ArgumentList '%BUILD_ID%'; ^
            Fail-StageEvent -StageId 'configure' -Error 'CMake configure failed'; ^
            Complete-BuildEvent -Success $false -Summary @{stages_completed=0; stages_failed=1; total_warnings=0; total_errors=1; cache_hits=0; cache_misses=0; peak_memory_mb=0}; ^
            Close-EventWriter"
        exit /b 1
    )

    powershell -NoProfile -ExecutionPolicy Bypass -Command ^
        "Import-Module '%HERE%..\..\runtime\scripts\BuildEvents.psm1' -ArgumentList '%BUILD_ID%'; ^
        Progress-StageEvent -StageId 'configure' -Progress 1.0 -Message 'CMake configure completed'; ^
        Complete-StageEvent -StageId 'configure' -Success $true -Output 'CMake configure successful'; ^
        Start-StageEvent -StageId 'build' -StageName 'Build (%GAME%)' -StageIndex 2 -TotalStages 2; ^
        Progress-StageEvent -StageId 'build' -Progress 0.1 -Message 'Starting Ninja build with %NX_JOBS% jobs'; ^
        " ^
        2>&1

    call "%HERE%nx_env.cmd" cmake --build "%NX_BUILD%" --target %GAME%_modules -- -j %NX_JOBS%
    if errorlevel 1 (
        powershell -NoProfile -ExecutionPolicy Bypass -Command ^
            "Import-Module '%HERE%..\..\runtime\scripts\BuildEvents.psm1' -ArgumentList '%BUILD_ID%'; ^
            Fail-StageEvent -StageId 'build' -Error 'Ninja build failed'; ^
            Complete-BuildEvent -Success $false -Summary @{stages_completed=1; stages_failed=1; total_warnings=0; total_errors=1; cache_hits=0; cache_misses=0; peak_memory_mb=0}; ^
            Close-EventWriter"
        exit /b 1
    )

    powershell -NoProfile -ExecutionPolicy Bypass -Command ^
        "Import-Module '%HERE%..\..\runtime\scripts\BuildEvents.psm1' -ArgumentList '%BUILD_ID%'; ^
        Progress-StageEvent -StageId 'build' -Progress 1.0 -Message 'Ninja build completed'; ^
        Complete-StageEvent -StageId 'build' -Success $true -Output 'Build successful'; ^
        Complete-BuildEvent -Success $true -Summary @{stages_completed=2; stages_failed=0; total_warnings=0; total_errors=0; cache_hits=0; cache_misses=0; peak_memory_mb=0}; ^
        Close-EventWriter" ^
        2>&1

    exit /b 0
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
if not defined NX_JOBS set /a NX_JOBS=%NUMBER_OF_PROCESSORS%-2
if not defined NX_JOBS set "NX_JOBS=1"
if %NX_JOBS% LSS 1 set "NX_JOBS=1"
if %NX_JOBS% GTR 16 set "NX_JOBS=16"
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
