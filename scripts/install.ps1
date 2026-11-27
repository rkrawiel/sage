<#
.SYNOPSIS
    Installs Sage for the current user: no admin rights, no extra tools.

.DESCRIPTION
    Copies sage.exe and uninstall.ps1 to %LOCALAPPDATA%\Programs\Sage, adds a
    "Sage" shortcut to the Start menu, and registers, under HKCU only:
      - "Open with Sage" on folders and on a folder's background (opens it as a vault);
      - Sage in the "Open with" list of .md files (never as their default app);
      - Sage's app id, so its task reminders show as Windows notifications;
      - an entry in Settings > Apps that runs uninstall.ps1.
    Run it again to update Sage. -WhatIf shows every change without making it.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -WhatIf
#>
[CmdletBinding(SupportsShouldProcess)]
param(
    # The sage.exe to install; by default the release build of this checkout.
    [string]$Exe
)

$ErrorActionPreference = 'Stop'

# Not a parameter default: Windows PowerShell leaves $PSScriptRoot empty there.
if (-not $Exe) {
    $Exe = Join-Path $PSScriptRoot '..\target\release\sage.exe'
}
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) {
    throw "No sage.exe at $Exe. Build it with: cargo build --release -p sage-app"
}
$Exe = (Resolve-Path -LiteralPath $Exe).Path

# The workspace version, for Settings > Apps.
$cargo = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\Cargo.toml')
if ($cargo -notmatch '(?ms)^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"') {
    throw 'No version in [workspace.package] of Cargo.toml'
}
$version = $Matches[1]

$dir = Join-Path $env:LOCALAPPDATA 'Programs\Sage'
$target = Join-Path $dir 'sage.exe'
$uninstaller = Join-Path $dir 'uninstall.ps1'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Sage.lnk'
$icon = "$target,0"
$classes = 'Software\Classes'

# Writes one value under HKCU, creating the key; '' names the default value.
function Set-UserValue([string]$Key, [string]$Name, $Value, [string]$Kind = 'String') {
    $label = if ($Name) { $Name } else { '(default)' }
    if ($PSCmdlet.ShouldProcess("HKCU\$Key", "Set $label = $Value")) {
        $handle = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($Key)
        try { $handle.SetValue($Name, $Value, [Microsoft.Win32.RegistryValueKind]$Kind) }
        finally { $handle.Dispose() }
    }
}

# Files.
if (-not (Test-Path -LiteralPath $dir)) {
    New-Item -ItemType Directory -Path $dir | Out-Null
}
Copy-Item -LiteralPath $Exe -Destination $target -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'uninstall.ps1') -Destination $uninstaller -Force

# Start menu.
if ($PSCmdlet.ShouldProcess($shortcut, 'Create shortcut to Sage')) {
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
    $link.TargetPath = $target
    $link.WorkingDirectory = $dir
    $link.IconLocation = $icon
    $link.Description = 'Markdown notes'
    $link.Save()
}

# "Open with Sage" on a folder, and inside one (%V is the folder shown).
foreach ($entry in @(@('Directory\shell\Sage', '%1'), @('Directory\Background\shell\Sage', '%V'))) {
    $key = "$classes\$($entry[0])"
    Set-UserValue $key '' 'Open with Sage'
    Set-UserValue $key 'Icon' $icon
    Set-UserValue "$key\command" '' "`"$target`" `"$($entry[1])`""
}

# .md files: Sage in "Open with", never the default.
Set-UserValue "$classes\.md\OpenWithProgids" 'Sage.Markdown' ''
Set-UserValue "$classes\Sage.Markdown" '' 'Markdown note'
Set-UserValue "$classes\Sage.Markdown\DefaultIcon" '' $icon
Set-UserValue "$classes\Sage.Markdown\shell\open\command" '' "`"$target`" `"%1`""
Set-UserValue "$classes\Applications\sage.exe" 'FriendlyAppName' 'Sage'
Set-UserValue "$classes\Applications\sage.exe\DefaultIcon" '' $icon
Set-UserValue "$classes\Applications\sage.exe\SupportedTypes" '.md' ''
Set-UserValue "$classes\Applications\sage.exe\shell\open\command" '' "`"$target`" `"%1`""

# Toasts for task reminders: Windows shows them only for a registered app id.
Set-UserValue "$classes\AppUserModelId\Sage.Sage" 'DisplayName' 'Sage'

# Settings > Apps.
$uninstall = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Sage'
Set-UserValue $uninstall 'DisplayName' 'Sage'
Set-UserValue $uninstall 'DisplayIcon' $icon
Set-UserValue $uninstall 'DisplayVersion' $version
Set-UserValue $uninstall 'Publisher' 'Sage'
Set-UserValue $uninstall 'InstallLocation' $dir
Set-UserValue $uninstall 'UninstallString' "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$uninstaller`""
Set-UserValue $uninstall 'NoModify' 1 'DWord'
Set-UserValue $uninstall 'NoRepair' 1 'DWord'

if (-not $WhatIfPreference) {
    Write-Host "Sage $version is installed in $dir"
}
