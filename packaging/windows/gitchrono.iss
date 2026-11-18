; Windows installer, built by .github/workflows/release.yml:
;   iscc /DVersion=0.9.0 /DSource=dist\GitChrono-0.9.0-windows-x86_64 /DOutput=dist packaging\windows\gitchrono.iss
; Installs for the current user only (no admin prompt) into
; %LOCALAPPDATA%\Programs\GitChrono, where GitChrono can update itself.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #error Pass /DSource=<folder with gitchrono.exe and mingit\>
#endif
#ifndef Output
  #define Output "."
#endif

[Setup]
AppId={{6F1C2E5A-8B47-4C1D-9E3A-2D5B7F0C4A19}
AppName=GitChrono
AppVersion={#Version}
AppVerName=GitChrono {#Version}
AppPublisher=GitChrono
AppPublisherURL=https://github.com/KumaVolt/GitChrono
AppSupportURL=https://github.com/KumaVolt/GitChrono/issues
DefaultDirName={autopf}\GitChrono
DefaultGroupName=GitChrono
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Output}
OutputBaseFilename=GitChrono-{#Version}-windows-x86_64-setup
SetupIconFile=gitchrono.ico
UninstallDisplayIcon={app}\gitchrono.exe
UninstallDisplayName=GitChrono
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\gitchrono.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\mingit\*"; DestDir: "{app}\mingit"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\GitChrono"; Filename: "{app}\gitchrono.exe"
Name: "{autodesktop}\GitChrono"; Filename: "{app}\gitchrono.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\gitchrono.exe"; Description: "{cm:LaunchProgram,GitChrono}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; What in-app updates leave next to the exe (gitchrono.old.exe, mingit.old).
Type: filesandordirs; Name: "{app}"
