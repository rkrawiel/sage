; Inno Setup 6 script for Sage: a per-user setup.exe that needs no admin rights.
;
; Build sage.exe first, then compile this script from the repository root:
;   cargo build --release -p sage-app
;   iscc installer\sage.iss
; Inno Setup itself: winget install JRSoftware.InnoSetup
; The result is target\installer\Sage-<version>-setup.exe.
;
; It installs what scripts\install.ps1 does: sage.exe in
; %LOCALAPPDATA%\Programs\Sage, a Start-menu entry, "Open with Sage" on
; folders and in .md files' "Open with" list (never as their default app),
; all under HKCU, plus an optional desktop icon. Its own uninstaller removes it.

#define AppName "Sage"
; SourcePath is this script's folder, so iscc can run from anywhere.
#define SageExe SourcePath + "..\target\release\sage.exe"
; sage.exe carries the workspace version from Cargo.toml (winresource sets
; ProductVersion to CARGO_PKG_VERSION). Override with iscc /DAppVersion=x.y.z.
#ifndef AppVersion
  #define AppVersion GetStringFileInfo(SageExe, "ProductVersion")
#endif

[Setup]
; Never change: Windows knows this installation by it.
AppId={{BA3F6C20-E22E-4358-9765-FA8EFDB2D799}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Sage
PrivilegesRequired=lowest
; With PrivilegesRequired=lowest, {autopf} is %LOCALAPPDATA%\Programs.
DefaultDirName={autopf}\Sage
DisableProgramGroupPage=yes
DisableDirPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\assets\icons\sage.ico
UninstallDisplayIcon={app}\sage.exe
UninstallDisplayName={#AppName}
ChangesAssociations=yes
OutputDir=..\target\installer
OutputBaseFilename=Sage-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SageExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Sage"; Filename: "{app}\sage.exe"; Comment: "Markdown notes"
Name: "{autodesktop}\Sage"; Filename: "{app}\sage.exe"; Comment: "Markdown notes"; Tasks: desktopicon

[Registry]
; "Open with Sage" on a folder: opens it as a vault.
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Sage"; ValueType: string; ValueName: ""; ValueData: "Open with Sage"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Sage"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\sage.exe,0"
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Sage\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sage.exe"" ""%1"""
; ... and on a folder's background (%V is the folder shown).
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Sage"; ValueType: string; ValueName: ""; ValueData: "Open with Sage"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Sage"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\sage.exe,0"
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Sage\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sage.exe"" ""%V"""
; .md files: Sage in "Open with", never the default. Only the value goes on uninstall.
Root: HKCU; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "Sage.Markdown"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\Sage.Markdown"; ValueType: string; ValueName: ""; ValueData: "Markdown note"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Sage.Markdown\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\sage.exe,0"
Root: HKCU; Subkey: "Software\Classes\Sage.Markdown\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sage.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\sage.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Sage"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\sage.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\sage.exe,0"
Root: HKCU; Subkey: "Software\Classes\Applications\sage.exe\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\sage.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sage.exe"" ""%1"""
; Toasts for task reminders: Windows shows them only for a registered app id.
Root: HKCU; Subkey: "Software\Classes\AppUserModelId\Sage.Sage"; ValueType: string; ValueName: "DisplayName"; ValueData: "Sage"; Flags: uninsdeletekey
; Settings > Apps: Inno Setup writes its own entry (with NoModify and
; NoRepair) under ...\Uninstall\{AppId}_is1.
