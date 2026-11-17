$env:DISABLE_VK_LAYER_reshade_1 = '1'
$env:DISABLE_VK_LAYER_reshade = '1'
$exe = 'C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\engine\fefw-recomp\game-build\fefw\build\release\fefw.exe'
$wd = Join-Path $env:APPDATA 'EmblemWeave\play'
$p = Start-Process -FilePath $exe -WorkingDirectory $wd -PassThru
Start-Sleep -Seconds 40
if (-not $p.HasExited) {
  Write-Output 'still running; stopping it now'
  $p.Kill()
} else {
  Write-Output ('exited with ' + $p.ExitCode)
}
