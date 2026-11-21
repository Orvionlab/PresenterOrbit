[CmdletBinding(DefaultParameterSetName = 'Interactive')]
param(
  [Parameter(ParameterSetName = 'Interactive')]
  [Security.SecureString]$RegistrationToken,
  [Parameter(Mandatory = $true, ParameterSetName = 'File')]
  [string]$RegistrationTokenFile,
  [string]$ArchiveDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
  throw 'Run this bootstrap inside the disposable Windows x64 test VM.'
}
$taskComputer = Get-CimInstance Win32_ComputerSystem
if ($taskComputer.Manufacturer -ne 'Microsoft Corporation' -or $taskComputer.Model -ne 'Virtual Machine') {
  throw 'This bootstrap only runs inside a Hyper-V Windows guest, never on the operator host.'
}
$taskVmId = (Get-CimInstance Win32_ComputerSystemProduct).UUID
if (-not $taskVmId -or $taskVmId -eq 'FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF') {
  throw 'The Hyper-V guest identity could not be verified.'
}
$taskOfflineArchives = $null
if ($PSBoundParameters.ContainsKey('ArchiveDirectory')) {
  $taskArchiveDirectory = Get-Item -LiteralPath $ArchiveDirectory
  if (-not $taskArchiveDirectory.PSIsContainer -or ($taskArchiveDirectory.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'ArchiveDirectory must name the explicitly supplied ordinary directory containing all verified bootstrap archives.'
  }
  $taskOfflineArchives = $taskArchiveDirectory.FullName
}
$taskIdentity = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$taskRepository = 'https://github.com/Joelsuperstar/Presenter-Orbit'
$taskRunnerVersion = '2.337.0'
$taskRunnerHash = '1150692afa94e71f872017e254ea55b6eece1eece3fe7e3a6d4c93d0a1b85cfc'
$taskRunnerRoot = Join-Path ([IO.Path]::GetTempPath()) 'orbit-package-test-runner-v1'
$taskRunnerRoot = [IO.Path]::GetFullPath($taskRunnerRoot)
if ([IO.Path]::GetDirectoryName($taskRunnerRoot).TrimEnd('\') -ne [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') -or $taskRunnerRoot -match '\s') {
  throw 'The disposable guest TEMP directory must have an absolute path without whitespace for silent NSIS acceptance.'
}

$taskOwnerFile = Join-Path $taskRunnerRoot 'orbit-test-runner-owner.json'
if (Test-Path -LiteralPath $taskRunnerRoot) {
  if (-not (Test-Path -LiteralPath $taskOwnerFile)) { throw 'The existing temporary runner directory has no ownership marker; it will not be modified.' }
  if ((Get-Item -LiteralPath $taskRunnerRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'The runner directory must not be a filesystem link.' }
  $taskOwner = Get-Content -LiteralPath $taskOwnerFile -Raw | ConvertFrom-Json
  if ($taskOwner.repository -ne $taskRepository -or $taskOwner.sid -ne $taskIdentity -or $taskOwner.vmId -ne $taskVmId) {
    throw 'The existing runner directory belongs to another profile, VM, or repository.'
  }
} else {
  New-Item -ItemType Directory -Path $taskRunnerRoot | Out-Null
  @{ repository = $taskRepository; sid = $taskIdentity; vmId = $taskVmId } | ConvertTo-Json | Set-Content -LiteralPath $taskOwnerFile -Encoding UTF8
}
$taskListenerPath = Join-Path $taskRunnerRoot 'bin/Runner.Listener.exe'
$taskExistingListener = Get-CimInstance Win32_Process -Filter "Name = 'Runner.Listener.exe'" | Where-Object { $_.ExecutablePath -eq $taskListenerPath }
if ($taskExistingListener) {
  Write-Output 'The disposable guest runner is already running; no duplicate listener was started.'
  return
}

function Get-VerifiedGuestArchive {
  param([string]$Name, [string]$Url, [string]$Hash, [string]$Algorithm = 'SHA256')
  $taskDownload = Join-Path $taskRunnerRoot $Name
  if ($taskOfflineArchives) {
    $taskSuppliedArchive = Get-Item -LiteralPath (Join-Path $taskOfflineArchives $Name)
    if ($taskSuppliedArchive.PSIsContainer -or ($taskSuppliedArchive.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
      throw "The explicitly supplied $Name archive must be an ordinary file."
    }
    if ((Get-FileHash -LiteralPath $taskSuppliedArchive.FullName -Algorithm $Algorithm).Hash.ToLowerInvariant() -ne $Hash) {
      throw "The supplied $Name archive failed pinned $Algorithm verification; it will not replace the guest cache."
    }
    if (Test-Path -LiteralPath $taskDownload) {
      if ((Get-Item -LiteralPath $taskDownload).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'The guest archive cache must not contain filesystem links.'
      }
    }
    if ($taskSuppliedArchive.FullName -ne $taskDownload) {
      Copy-Item -LiteralPath $taskSuppliedArchive.FullName -Destination $taskDownload -Force
    }
  } elseif (-not (Test-Path -LiteralPath $taskDownload)) {
    Write-Host "Downloading $Name from its official publisher."
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    try { Invoke-WebRequest -Uri $Url -OutFile $taskDownload -UseBasicParsing -TimeoutSec 120 }
    catch { throw "Downloading $Name from $Url failed: $($_.Exception.Message). Check guest HTTPS/proxy connectivity or rerun with the explicitly prepared -ArchiveDirectory kit; no alternate download source was used." }
  }
  if ((Get-FileHash -LiteralPath $taskDownload -Algorithm $Algorithm).Hash.ToLowerInvariant() -ne $Hash) {
    throw "The pinned $Algorithm verification failed for $Name. Remove only that archive inside this guest and retry."
  }
  Write-Host "Verified $Name ($Algorithm)."
  return $taskDownload
}

$taskTools = Join-Path $taskRunnerRoot 'tools'
New-Item -ItemType Directory -Path $taskTools -Force | Out-Null
$taskNodeRoot = Join-Path $taskTools 'node-v26.5.1-win-x64'
$taskNodeArchive = Get-VerifiedGuestArchive -Name 'node-v26.5.1-win-x64.zip' -Url 'https://nodejs.org/dist/v26.5.1/node-v26.5.1-win-x64.zip' -Hash 'c432c996b95cbf7568f13a0fbb37526de84a27e3a5c520c3be15f05a9a168212'
if (-not (Test-Path -LiteralPath (Join-Path $taskNodeRoot 'node.exe'))) {
  Expand-Archive -LiteralPath $taskNodeArchive -DestinationPath $taskTools -Force
}
$taskPwshRoot = Join-Path $taskTools 'powershell-7.4.21'
$taskPwshArchive = Get-VerifiedGuestArchive -Name 'PowerShell-7.4.21-win-x64.zip' -Url 'https://github.com/PowerShell/PowerShell/releases/download/v7.4.21/PowerShell-7.4.21-win-x64.zip' -Hash 'b47ee814b9fe5fe6b713a26667998cadd6bbb74cb3889fadea56c775f11953a2'
if (-not (Test-Path -LiteralPath (Join-Path $taskPwshRoot 'pwsh.exe'))) {
  Expand-Archive -LiteralPath $taskPwshArchive -DestinationPath $taskPwshRoot -Force
}
$taskGitRoot = Join-Path $taskTools 'mingit-2.56.0.2'
$taskGitArchive = Get-VerifiedGuestArchive -Name 'MinGit-2.56.0.2-64-bit.zip' -Url 'https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.2/MinGit-2.56.0.2-64-bit.zip' -Hash 'da35e72aa21c005a5a0d298cfbae110bc1609a815730ea0dde84b01a1b3cd3be'
if (-not (Test-Path -LiteralPath (Join-Path $taskGitRoot 'cmd/git.exe'))) {
  Expand-Archive -LiteralPath $taskGitArchive -DestinationPath $taskGitRoot -Force
}
$taskNpmPrefix = Join-Path $taskRunnerRoot 'npm-global'
New-Item -ItemType Directory -Path $taskNpmPrefix -Force | Out-Null
$env:NPM_CONFIG_PREFIX = $taskNpmPrefix
$env:PATH = "$taskNpmPrefix;$taskNodeRoot;$taskPwshRoot;$(Join-Path $taskGitRoot 'cmd');$env:PATH"
$taskNpmIntegrity = 'uIXokLlBj6FpNUTQX1PmT5pz7BlIN9QlixX+zdaSNHsd0qUXsbDLr50xzY6Sw7cJVr0uzHKDOle0swmPW/p5Qw=='
$taskNpmHash = -join ([Convert]::FromBase64String($taskNpmIntegrity) | ForEach-Object { $_.ToString('x2') })
$taskNpmArchive = Get-VerifiedGuestArchive -Name 'npm-12.0.2.tgz' -Url 'https://registry.npmjs.org/npm/-/npm-12.0.2.tgz' -Hash $taskNpmHash -Algorithm SHA512
if (-not (Test-Path -LiteralPath (Join-Path $taskNpmPrefix 'npm.cmd'))) {
  & (Join-Path $taskNodeRoot 'npm.cmd') install --global --prefix $taskNpmPrefix $taskNpmArchive --ignore-scripts --offline --no-audit --no-fund
  if ($LASTEXITCODE -ne 0) { throw 'Installing the verified portable npm package failed inside the guest.' }
}
if ((& node.exe --version).Trim() -ne 'v26.5.1' -or $LASTEXITCODE -ne 0) { throw 'The guest requires Node.js 26.5.1.' }
if ((& npm.cmd --version).Trim() -ne '12.0.2' -or $LASTEXITCODE -ne 0) { throw 'The guest requires npm 12.0.2.' }
& git.exe --version
if ($LASTEXITCODE -ne 0) { throw 'The verified portable Git failed to start.' }
& pwsh.exe -NoProfile -NonInteractive -Command '$PSVersionTable.PSVersion.ToString()'
if ($LASTEXITCODE -ne 0) { throw 'The verified portable PowerShell failed to start.' }
Write-Output 'WebView2 will be provisioned by the actual silent NSIS installer if it is missing from this disposable guest.'

$taskArchive = Get-VerifiedGuestArchive -Name "actions-runner-win-x64-$taskRunnerVersion.zip" -Url "https://github.com/actions/runner/releases/download/v$taskRunnerVersion/actions-runner-win-x64-$taskRunnerVersion.zip" -Hash $taskRunnerHash
if (-not (Test-Path -LiteralPath $taskListenerPath)) {
  Expand-Archive -LiteralPath $taskArchive -DestinationPath $taskRunnerRoot -Force
}

$taskMetadataPath = Join-Path $taskRunnerRoot '.runner'
if (Test-Path -LiteralPath $taskMetadataPath) {
  $taskMetadata = Get-Content -LiteralPath $taskMetadataPath -Raw | ConvertFrom-Json
  if ($taskMetadata.gitHubUrl.TrimEnd('/') -ne $taskRepository -or $taskMetadata.ephemeral -ne $true) {
    throw 'The existing runner registration is not the expected ephemeral Presenter Orbit runner.'
  }
} else {
  if ($PSCmdlet.ParameterSetName -eq 'File') {
    $taskTokenFile = Get-Item -LiteralPath $RegistrationTokenFile
    if ($taskTokenFile.PSIsContainer -or $taskTokenFile.Name -ne 'orbit-registration-token.json' -or ($taskTokenFile.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
      throw 'Provide the generated orbit-registration-token.json file as an ordinary file inside the disposable guest.'
    }
    try { $taskTokenDocument = Get-Content -LiteralPath $taskTokenFile.FullName -Raw | ConvertFrom-Json }
    catch { throw 'The registration-token file could not be parsed; request a fresh generated VM kit.' }
    if ($taskTokenDocument.token -isnot [string] -or -not $taskTokenDocument.token -or [DateTimeOffset]::Parse($taskTokenDocument.expires_at) -le [DateTimeOffset]::UtcNow.AddMinutes(2)) {
      throw 'The registration-token file is invalid or expired; request a fresh generated VM kit.'
    }
    $RegistrationToken = ConvertTo-SecureString -String $taskTokenDocument.token -AsPlainText -Force
    $taskTokenDocument = $null
  }
  if (-not $RegistrationToken) { $RegistrationToken = Read-Host 'Paste the short-lived repository runner registration token (hidden)' -AsSecureString }
  if ($RegistrationToken.Length -eq 0) { throw 'A short-lived repository registration token is required.' }
  $taskTokenPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($RegistrationToken)
  try {
    $taskTokenPlain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($taskTokenPointer)
    Push-Location $taskRunnerRoot
    try {
      & .\config.cmd --unattended --url $taskRepository --token $taskTokenPlain --name "$env:COMPUTERNAME-orbit-package-test" --labels orbit-package-tests --work _work --ephemeral
      if ($LASTEXITCODE -ne 0) { throw 'Ephemeral runner registration failed. Request a fresh repository registration token and rerun.' }
      if ($PSCmdlet.ParameterSetName -eq 'File') { Remove-Item -LiteralPath $taskTokenFile.FullName }
    } finally { Pop-Location }
  } finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($taskTokenPointer)
    $taskTokenPlain = $null
    $RegistrationToken.Dispose()
    $RegistrationToken = $null
  }
}

$taskEnvironment = @(
  'PRESENTER_ORBIT_PACKAGE_TEST_PROFILE=disposable',
  "NPM_CONFIG_PREFIX=$taskNpmPrefix"
)
[IO.File]::WriteAllLines((Join-Path $taskRunnerRoot '.env'), $taskEnvironment, [Text.UTF8Encoding]::new($false))
Write-Output 'The disposable VM runner is ready for one GitHub Actions job. Keep this guest signed in and this window open.'
Write-Output 'The host build runners and host startup configuration are unchanged. Registration ends automatically after one job.'
Push-Location $taskRunnerRoot
try {
  & .\run.cmd
  if ($LASTEXITCODE -ne 0) { throw 'The disposable guest runner exited unsuccessfully; inspect its _diag logs inside the guest.' }
} finally { Pop-Location }
