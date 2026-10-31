; Inno Setup script for WoW-Tools-<version>-setup.exe (a per-user install, no admin rights).
; Built by release.yml and ci.yml:
;   ISCC.exe /DAppVersion=1.2.3 /DSourceExe=..\target\release\wow-tools.exe /O..\dist packaging\installer.iss
; The AppId is also in src\selfupdate.rs (UNINSTALL_KEY): the self-update keeps DisplayVersion current.

#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\wow-tools.exe"
#endif

[Setup]
AppId={{6E0A3C52-8B4F-4D1E-9C7A-2F5B8D3E1A47}
AppName=WoW Tools
AppVersion={#AppVersion}
AppVerName=WoW Tools {#AppVersion}
AppPublisher=Aleksandr Stepanov
AppPublisherURL=https://github.com/Gronck2/wow-tools
AppSupportURL=https://github.com/Gronck2/wow-tools/issues
AppUpdatesURL=https://github.com/Gronck2/wow-tools/releases
VersionInfoVersion={#AppVersion}
VersionInfoProductName=WoW Tools
; Per user: %LOCALAPPDATA%\Programs\WoW Tools, writable, so the app can update itself.
PrivilegesRequired=lowest
DefaultDirName={autopf}\WoW Tools
DisableDirPage=auto
DisableProgramGroupPage=yes
DisableReadyPage=yes
ShowLanguageDialog=no
UsedUserAreasWarning=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=WoW-Tools-{#AppVersion}-setup
SetupIconFile=..\assets\wow-tools.ico
UninstallDisplayIcon={app}\wow-tools.exe
UninstallDisplayName=WoW Tools
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The tray is closed by `wow-tools quit` (PrepareToInstall), which waits for a running job.
CloseApplications=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[CustomMessages]
en.Autostart=Start with Windows
ru.Autostart=Запускать вместе с Windows
de.Autostart=Mit Windows starten
fr.Autostart=Démarrer avec Windows
es.Autostart=Iniciar con Windows
en.Busy=WoW Tools is busy (a backup or an addon update). Try again when it is done.
ru.Busy=WoW Tools занят (бэкап или обновление аддонов). Повторите, когда он закончит.
de.Busy=WoW Tools ist beschäftigt (Sicherung oder Addon-Update). Versuchen Sie es danach erneut.
fr.Busy=WoW Tools est occupé (sauvegarde ou mise à jour des addons). Réessayez une fois terminé.
es.Busy=WoW Tools está ocupado (una copia o una actualización de addons). Inténtalo de nuevo cuando termine.
en.DeleteData=Also delete the settings and the backups?%n%n%1%n%2%n%nA backup folder you chose yourself is not touched.
ru.DeleteData=Удалить также настройки и бэкапы?%n%n%1%n%2%n%nПапку бэкапов, выбранную вами вручную, это не затронет.
de.DeleteData=Auch die Einstellungen und Sicherungen löschen?%n%n%1%n%2%n%nEin selbst gewählter Sicherungsordner bleibt unberührt.
fr.DeleteData=Supprimer aussi les paramètres et les sauvegardes ?%n%n%1%n%2%n%nUn dossier de sauvegarde choisi par vous n'est pas touché.
es.DeleteData=¿Eliminar también la configuración y las copias?%n%n%1%n%2%n%nUna carpeta de copias elegida por ti no se toca.

[Tasks]
Name: "autostart"; Description: "{cm:Autostart}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "wow-tools.exe"; Flags: ignoreversion
Source: "..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\WoW Tools"; Filename: "{app}\wow-tools.exe"
Name: "{autodesktop}\WoW Tools"; Filename: "{app}\wow-tools.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\wow-tools.exe"; Parameters: "schedule logon"; Flags: runhidden; Tasks: autostart
Filename: "{app}\wow-tools.exe"; Description: "{cm:LaunchProgram,WoW Tools}"; Flags: nowait postinstall skipifsilent runhidden

[UninstallRun]
Filename: "{app}\wow-tools.exe"; Parameters: "schedule remove"; Flags: runhidden; RunOnceId: "RemoveAutostart"

[UninstallDelete]
; Left by a self-update until the next start.
Type: files; Name: "{app}\wow-tools.exe.old"
Type: dirifempty; Name: "{app}"

[Code]
var
  TrayWasRunning: Boolean;

function TrayRunning(): Boolean;
begin
  Result := FindWindowByClassName('WowToolsTray') <> 0;
end;

{ Closes the running tray; False while it is busy (exit code 3). }
function QuitTray(Exe: String): Boolean;
var
  Code: Integer;
begin
  Result := True;
  if TrayRunning() and Exec(Exe, 'quit', '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Result := Code <> 3;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  TrayWasRunning := TrayRunning();
  if TrayWasRunning then
  begin
    { The new exe knows `quit` even when the running one is older. }
    ExtractTemporaryFile('wow-tools.exe');
    if not QuitTray(ExpandConstant('{tmp}\wow-tools.exe')) then
      Result := CustomMessage('Busy');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  { A silent upgrade (/SILENT, /VERYSILENT) has no "Launch" checkbox: start the tray again if it was running. }
  if (CurStep = ssPostInstall) and WizardSilent() and TrayWasRunning then
    Exec(ExpandConstant('{app}\wow-tools.exe'), '', '', SW_HIDE, ewNoWait, Code);
end;

function InitializeUninstall(): Boolean;
begin
  Result := QuitTray(ExpandConstant('{app}\wow-tools.exe'));
  if not Result then
    MsgBox(CustomMessage('Busy'), mbError, MB_OK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent() then
    if MsgBox(FmtMessage(CustomMessage('DeleteData'), [ExpandConstant('{userappdata}\WoW Tools'), ExpandConstant('{localappdata}\WoW Tools')]), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\WoW Tools'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\WoW Tools'), True, True, True);
    end;
end;
