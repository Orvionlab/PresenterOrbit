param(
  [ValidateSet('Both', 'Windows', 'Linux')]
  [string]$Platform = 'Both',
  [string]$WindowsRunnerRoot = (Join-Path $env:USERPROFILE 'actions-runner-presenter-orbit'),
  [string]$LinuxDistribution = 'Ubuntu-24.04'
)

$ErrorActionPreference = 'Stop'
$taskRunnerConfiguration = Join-Path $WindowsRunnerRoot '.runner'
if (-not (Test-Path -LiteralPath $taskRunnerConfiguration)) { throw 'The Windows runner has not been registered.' }
$taskRunnerMetadata = Get-Content -LiteralPath $taskRunnerConfiguration -Raw | ConvertFrom-Json
if ($taskRunnerMetadata.gitHubUrl.TrimEnd('/') -ne 'https://github.com/Joelsuperstar/Presenter-Orbit') {
  throw 'This launcher only starts runners registered for Presenter Orbit.'
}

if ($Platform -in @('Both', 'Windows')) {
  $taskListenerPath = Join-Path $WindowsRunnerRoot 'bin/Runner.Listener.exe'
  $taskListener = Get-CimInstance Win32_Process -Filter "Name = 'Runner.Listener.exe'" |
    Where-Object { $_.ExecutablePath -eq $taskListenerPath }
  if ($taskListener) {
    Write-Output 'Windows runner is already running.'
  } else {
    $taskPreviousPath = $env:PATH
    try {
      $env:PATH = 'C:\Program Files\Git\bin;' + $env:PATH
      $taskProcess = Start-Process -FilePath $env:ComSpec -ArgumentList @('/d', '/c', 'run.cmd') -WorkingDirectory $WindowsRunnerRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $WindowsRunnerRoot 'listener.stdout.log') -RedirectStandardError (Join-Path $WindowsRunnerRoot 'listener.stderr.log') -PassThru
      Set-Content -LiteralPath (Join-Path $WindowsRunnerRoot 'listener.pid') -Value $taskProcess.Id
      Write-Output 'Windows runner started.'
    } finally { $env:PATH = $taskPreviousPath }
  }
}

if ($Platform -in @('Both', 'Linux')) {
  $taskLinuxHome = (wsl -d $LinuxDistribution --exec printenv HOME).Trim()
  if ($LASTEXITCODE -ne 0 -or -not $taskLinuxHome.StartsWith('/')) { throw 'The Linux runner home is unavailable.' }
  $taskLinuxLauncher = "$taskLinuxHome/actions-runner-presenter-orbit-linux/start-runner.sh"
  wsl -d $LinuxDistribution --exec test -f $taskLinuxLauncher
  if ($LASTEXITCODE -ne 0) { throw 'The Linux runner launcher is missing.' }
  $taskLinuxListener = Get-CimInstance Win32_Process -Filter "Name = 'wsl.exe'" |
    Where-Object { $_.CommandLine -and $_.CommandLine.Contains($taskLinuxLauncher) }
  if ($taskLinuxListener) {
    Write-Output 'Linux runner is already running.'
  } else {
    $taskProcess = Start-Process -FilePath (Join-Path $env:WINDIR 'System32/wsl.exe') -ArgumentList @('-d', $LinuxDistribution, '--exec', 'bash', $taskLinuxLauncher) -WindowStyle Hidden -RedirectStandardOutput (Join-Path $WindowsRunnerRoot 'linux.stdout.log') -RedirectStandardError (Join-Path $WindowsRunnerRoot 'linux.stderr.log') -PassThru
    Set-Content -LiteralPath (Join-Path $WindowsRunnerRoot 'linux-listener.pid') -Value $taskProcess.Id
    Write-Output 'Linux runner started.'
  }
}
