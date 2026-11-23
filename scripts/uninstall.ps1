<#
.SYNOPSIS
    Removes what install.ps1 added for the current user, and nothing else.

.DESCRIPTION
    Deletes sage.exe and this script from %LOCALAPPDATA%\Programs\Sage (and
    the folder, if that leaves it empty), the Start-menu shortcut, and the
    HKCU keys and the one .md "Open with" value that install.ps1 wrote.
    Vaults, their .sage folders and Sage's index cache are left alone.
    -WhatIf shows every change without making it.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\Programs\Sage\uninstall.ps1"
#>
[CmdletBinding(SupportsShouldProcess)]
param()

$ErrorActionPreference = 'Stop'

$dir = Join-Path $env:LOCALAPPDATA 'Programs\Sage'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Sage.lnk'
$classes = 'Software\Classes'

function Remove-UserKey([string]$Key) {
    $root = [Microsoft.Win32.Registry]::CurrentUser
    $handle = $root.OpenSubKey($Key)
    if ($handle) {
        $handle.Dispose()
        if ($PSCmdlet.ShouldProcess("HKCU\$Key", 'Delete key')) {
            $root.DeleteSubKeyTree($Key)
        }
    }
}

# sage.exe first: if Sage is still running this stops here, with nothing removed.
foreach ($file in @((Join-Path $dir 'sage.exe'), $shortcut, (Join-Path $dir 'uninstall.ps1'))) {
    if (Test-Path -LiteralPath $file) {
        Remove-Item -LiteralPath $file
    }
}
if ((Test-Path -LiteralPath $dir) -and -not (Get-ChildItem -LiteralPath $dir -Force)) {
    Remove-Item -LiteralPath $dir
}

Remove-UserKey "$classes\Directory\shell\Sage"
Remove-UserKey "$classes\Directory\Background\shell\Sage"
Remove-UserKey "$classes\Sage.Markdown"
Remove-UserKey "$classes\Applications\sage.exe"
Remove-UserKey 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Sage'

# Only Sage's own value; .md and its other "Open with" apps stay.
$progids = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$classes\.md\OpenWithProgids", $true)
if ($progids) {
    try {
        if ($progids.GetValueNames() -contains 'Sage.Markdown' -and
            $PSCmdlet.ShouldProcess("HKCU\$classes\.md\OpenWithProgids", 'Delete value Sage.Markdown')) {
            $progids.DeleteValue('Sage.Markdown')
        }
    }
    finally { $progids.Dispose() }
}

if (-not $WhatIfPreference) {
    Write-Host 'Sage is uninstalled.'
}
