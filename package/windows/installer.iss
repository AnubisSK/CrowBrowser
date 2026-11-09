; CrowBrowser Windows installer (Inno Setup 6). Built by scripts\build_windows_installer.ps1,
; which passes /DAppVersion from Cargo.toml. Manual: ISCC.exe package\windows\installer.iss
;
; Per-user install without administrator rights ({autopf} = %LOCALAPPDATA%\Programs); the
; wizard offers an all-users install (Program Files) when elevated. The WebView2 loader is
; linked statically into crow-browser.exe; the WebView2 Runtime itself ships with Windows 10/11.

#ifndef AppVersion
  #define AppVersion "0.1.3"
#endif
#define AppName "CrowBrowser"
#define AppExe "crow-browser.exe"
#define Root "..\.."
#define Bin Root + "\target\x86_64-pc-windows-msvc\release\" + AppExe

[Setup]
AppId={{6B0C5E3A-8F1D-4C2B-9A77-2D3E4F5C6B71}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=CrowBrowser
AppPublisherURL=https://github.com/AnubisSK/CrowBrowser
AppUpdatesURL=https://github.com/AnubisSK/CrowBrowser/releases
AppCopyright=CrowBrowser Team
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Root}\dist
OutputBaseFilename=crow-browser-setup-v{#AppVersion}
SetupIconFile={#Root}\assets\crow.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
LZMAUseSeparateProcess=yes
; Modern wizard; follows the system dark mode where the compiler supports it (Inno Setup 6.6+).
#if Ver >= EncodeVer(6, 6, 0)
WizardStyle=modern dynamic
#else
WizardStyle=modern
#endif
WizardSizePercent=110
CloseApplications=yes
RestartApplications=no
ShowLanguageDialog=auto

[Languages]
#ifexist "compiler:Languages\Slovak.isl"
Name: "sk"; MessagesFile: "compiler:Languages\Slovak.isl"
#endif
Name: "cs"; MessagesFile: "compiler:Languages\Czech.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Bin}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\assets\crow.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\crow.ico"; Comment: "{#AppName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\crow.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

; The encrypted vault (%APPDATA%\Crow) stays on uninstall: it is only readable with the master
; password and lets a reinstall pick up the session. Browsing data is never on disk (InPrivate).

[Code]
const
  WebView2Id = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

function WebView2Installed(): Boolean;
var
  Version: String;
begin
  Result :=
    (RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\' + WebView2Id, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKLM, 'SOFTWARE\Microsoft\EdgeUpdate\Clients\' + WebView2Id, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\' + WebView2Id, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

// CrowBrowser renders with the Microsoft Edge WebView2 Runtime (preinstalled on Windows 11 and
// current Windows 10). Nothing is downloaded by the installer; a missing runtime is pointed out.
function InitializeSetup(): Boolean;
var
  Code: Integer;
begin
  Result := True;
  if not WebView2Installed() then
    if SuppressibleMsgBox('CrowBrowser needs the Microsoft Edge WebView2 Runtime, which is not installed.' + #13#10#13#10 +
        'Open the download page now? (Setup continues either way.)', mbConfirmation, MB_YESNO, IDNO) = IDYES then
      ShellExec('open', 'https://developer.microsoft.com/microsoft-edge/webview2/', '', '', SW_SHOWNORMAL, ewNoWait, Code);
end;
