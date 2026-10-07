#ifndef AppVersion
  #define AppVersion "0.1.5"
#endif
#ifndef SourceRoot
  #define SourceRoot ".."
#endif

[Setup]
AppId={{A7AF1A47-7EE8-4C83-9B86-9C2D4AB38B53}
AppName=Shard Typer
AppVersion={#AppVersion}
AppPublisher=Shard Typer
DefaultDirName={localappdata}\Programs\ShardTyper
DefaultGroupName=Shard Typer
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=ShardTyper-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile={#SourceRoot}\assets\shard.ico
UninstallDisplayIcon={app}\ShardTyper.exe
AppMutex=Local\ShardTyper.v1
CloseApplications=yes
RestartApplications=no
LicenseFile={#SourceRoot}\LICENSE

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceRoot}\target\release\ShardTyper.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\QUICKSTART.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\THIRD_PARTY_NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Shard Typer"; Filename: "{app}\ShardTyper.exe"
Name: "{autodesktop}\Shard Typer"; Filename: "{app}\ShardTyper.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\ShardTyper.exe"; Description: "Open Shard Typer"; Flags: nowait postinstall skipifsilent

