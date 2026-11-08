; Inno Setup script for the Network Tester Windows app.
; Build it with:  powershell -File installer\build-installer.ps1
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppName "Network Tester"
#define AppExe "network-tester.exe"

[Setup]
; Do not change this ID. Windows uses it to find the old version when you install an update.
AppId={{4EBE521A-A6B0-45BC-9CE6-7CF3611040C1}
AppName={#AppName}
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Installs for the current user and needs no admin rights. The wizard offers an all-users install.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=NetworkTester-Setup-{#AppVersion}
SetupIconFile=..\crates\client\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName}"; Flags: nowait postinstall skipifsilent

; Test results are kept in %LOCALAPPDATA%\NetworkTester\results. Uninstall leaves them alone.
