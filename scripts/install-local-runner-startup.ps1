param(
  [string]$WindowsRunnerRoot = (Join-Path $env:USERPROFILE 'actions-runner-presenter-orbit')
)

$ErrorActionPreference = 'Stop'
$taskMetadataPath = Join-Path $WindowsRunnerRoot '.runner'
$taskMetadata = Get-Content -LiteralPath $taskMetadataPath -Raw | ConvertFrom-Json
if ($taskMetadata.gitHubUrl.TrimEnd('/') -ne 'https://github.com/Joelsuperstar/Presenter-Orbit') {
  throw 'Startup can only be installed for the registered Presenter Orbit runner.'
}
$taskSource = Join-Path $PSScriptRoot 'start-local-actions-runners.ps1'
$taskLauncher = Join-Path $WindowsRunnerRoot 'start-local-actions-runners.ps1'
$taskPowerShell = Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe'
$taskStartup = [Environment]::GetFolderPath('Startup')
if (-not $taskStartup -or -not (Test-Path -LiteralPath $taskStartup)) {
  throw 'The current Windows user Startup folder is unavailable.'
}
$taskShortcutPath = Join-Path $taskStartup 'Presenter Orbit Actions Runners.lnk'
$taskShell = New-Object -ComObject WScript.Shell
$taskShortcut = $taskShell.CreateShortcut($taskShortcutPath)
if ((Test-Path -LiteralPath $taskShortcutPath) -and
    ($taskShortcut.TargetPath -ne $taskPowerShell -or -not $taskShortcut.Arguments.Contains($taskLauncher))) {
  throw 'An unrelated shortcut already occupies the Presenter Orbit runner startup path.'
}
Copy-Item -LiteralPath $taskSource -Destination $taskLauncher -Force
$taskShortcut.TargetPath = $taskPowerShell
$taskShortcut.Arguments = '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + $taskLauncher + '" -WindowsRunnerRoot "' + $WindowsRunnerRoot + '"'
$taskShortcut.WorkingDirectory = $WindowsRunnerRoot
$taskShortcut.WindowStyle = 7
$taskShortcut.Description = 'Start the registered Presenter Orbit Windows and WSL build runners after sign-in.'
$taskShortcut.Save()
Write-Output "Runner startup installed for the current user: $taskShortcutPath"
Write-Output 'Both runners will start hidden after Windows sign-in. No password or administrator service is required.'
