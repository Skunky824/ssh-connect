; Inno Setup script for ssh-connect
#define MyAppName "ssh-connect"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Claudio Salvai"
#define MyAppURL "https://github.com/Skunky824/ssh-connect"
#define MyAppExeName "ssh-connect.exe"

[Setup]
AppId={{D88DBCB4-09FC-4F89-8A6C-7A32E3850CB9}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=ssh-connect-setup
Compression=lzma
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
PrivilegesRequired=lowest
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"; GroupDescription: "Additional icons:"; Flags: unchecked
Name: "addtopath"; Description: "Add ssh-connect to PATH"; GroupDescription: "Additional tasks:"; Flags: checkedonce

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}')); Tasks: addtopath

[Code]
function NeedsAddPath(PathToAdd: string): Boolean;
var
	CurrentPath: string;
	Needle: string;
begin
	Result := True;
	if not RegQueryStringValue(HKCU, 'Environment', 'Path', CurrentPath) then
		Exit;

	Needle := ';' + Lowercase(PathToAdd) + ';';
	Result := Pos(Needle, ';' + Lowercase(CurrentPath) + ';') = 0;
end;
