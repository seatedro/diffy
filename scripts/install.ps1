#!/usr/bin/env pwsh
# Diffy installer for Windows.
#
#   powershell -c "irm https://raw.githubusercontent.com/seatedro/diffy/master/scripts/install.ps1 | iex"
#
# Parameters:
#   -Version v0.1.0     install a specific tag (default: latest)
#   -Silent             run the NSIS installer in silent mode

param(
  [String]$Version = "latest",
  [Switch]$Silent = $false
)

$ErrorActionPreference = "Stop"

$Repo    = "seatedro/diffy"
$AppName = "Diffy"

function Write-Info  { param([String]$msg) Write-Host "==> $msg" -ForegroundColor Green }
function Write-Warn  { param([String]$msg) Write-Host "!!  $msg" -ForegroundColor Yellow }
function Write-Err   { param([String]$msg) Write-Host "error: $msg" -ForegroundColor Red; exit 1 }
function Write-Hint  { param([String]$msg) Write-Host "    $msg" -ForegroundColor DarkGray }

$Arch = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').PROCESSOR_ARCHITECTURE
switch ($Arch) {
  "AMD64" { $Target = "x64" }
  "ARM64" { $Target = "aarch64" }
  default { Write-Err "unsupported architecture: $Arch" }
}

if ($Version -eq "latest") {
  Write-Info "resolving latest release"
  try {
    $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
                                -Headers @{ "User-Agent" = "diffy-installer" }
    $Version = $latest.tag_name
  } catch {
    Write-Err "could not determine latest release: $_"
  }
}

if ($Version -notmatch '^v\d') { $Version = "v$Version" }
$NumericVersion = $Version.TrimStart('v')

$AssetName = "${AppName}_${NumericVersion}_${Target}-setup.exe"
$AssetUrl  = "https://github.com/$Repo/releases/download/$Version/$AssetName"
$SumsUrl   = "https://github.com/$Repo/releases/download/$Version/SHA256SUMS"

$TmpDir = New-Item -ItemType Directory -Path (Join-Path $env:TEMP "diffy-install-$(Get-Random)")
$SetupPath = Join-Path $TmpDir $AssetName
$SumsPath  = Join-Path $TmpDir "SHA256SUMS"

try {
  Write-Info "downloading $AssetName ($Version)"
  try {
    # curl.exe is faster and shows a progress bar; Invoke-WebRequest falls back
    # when curl isn't available (unlikely on Win10+ but possible on stripped images).
    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
      & curl.exe "-#SfLo" "$SetupPath" "$AssetUrl"
      if ($LASTEXITCODE -ne 0) { throw "curl exited with code $LASTEXITCODE" }
    } else {
      $prev = $ProgressPreference; $ProgressPreference = "SilentlyContinue"
      Invoke-WebRequest -Uri $AssetUrl -OutFile $SetupPath -UseBasicParsing
      $ProgressPreference = $prev
    }
  } catch {
    Write-Err "download failed for $AssetUrl`n       is '$Version' a published release? https://github.com/$Repo/releases"
  }

  try {
    $prev = $ProgressPreference; $ProgressPreference = "SilentlyContinue"
    Invoke-WebRequest -Uri $SumsUrl -OutFile $SumsPath -UseBasicParsing
    $ProgressPreference = $prev

    Write-Info "verifying checksum"
    $expectedLine = Get-Content $SumsPath | Where-Object { $_ -match [Regex]::Escape($AssetName) + '\s*$' }
    if (-not $expectedLine) {
      Write-Warn "$AssetName not listed in SHA256SUMS; skipping verification"
    } else {
      $expected = ($expectedLine -split '\s+')[0].ToLower()
      $actual   = (Get-FileHash -Algorithm SHA256 -Path $SetupPath).Hash.ToLower()
      if ($expected -ne $actual) {
        Write-Err "checksum mismatch for $AssetName`n       expected: $expected`n       actual:   $actual"
      }
    }
  } catch {
    Write-Warn "SHA256SUMS not available for $Version; skipping verification"
  }

  Write-Info "running installer"
  $args = @()
  if ($Silent) { $args += "/S" }
  $proc = Start-Process -FilePath $SetupPath -ArgumentList $args -Wait -PassThru
  if ($proc.ExitCode -ne 0) {
    Write-Err "installer exited with code $($proc.ExitCode)"
  }

  Write-Host ""
  Write-Host "installed $AppName $Version" -ForegroundColor Green
  Write-Hint "launch it from the Start Menu, or run:"
  Write-Hint "  & `"$env:LOCALAPPDATA\$AppName\$AppName.exe`""
}
finally {
  Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}
