# WoW Tools

[English](README.md) · [Русский](README.ru.md) · **Deutsch** · [Français](README.fr.md) · [Español](README.es.md)

Ein kleines Windows-Programm für den Infobereich (Tray), das Ihre Einstellungen und Addons in World of Warcraft schützt:

- **Sicherungen** von `WTF` (Tastenbelegung, Makros, Chat, CVars, Addon-Einstellungen) und `Interface\AddOns` – nach
  jeder Spielsitzung oder nach Zeitplan, für jede installierte Spielversion.
- **Wiederherstellen mit einem Klick** auf jeden früheren Stand; der aktuelle Stand wird vorher gesichert, sodass sich
  jede Wiederherstellung rückgängig machen lässt.
- **Einstellungen zwischen Charakteren kopieren**: Tastenbelegung, Makros, Chatfenster, Fensteranordnung, aktivierte
  Addons, Addon-Einstellungen.
- **Addon-Updates** über den Addon-Manager, den Sie ohnehin nutzen – CurseForge, WowUp, WowUp-CF oder die Wago App –
  ohne Fenster gestartet, nach Zeitplan und nie, während Sie spielen. ElvUI, das keiner von ihnen führt, direkt von
  seiner offiziellen Website.
- **Spiel-Patches**: Aktualisiert Battle.net das Spiel, werden Einstellungen und Addons vor dem ersten Start auf der
  neuen Version gesichert, und Sie sehen, welche Addons noch nicht dafür gemacht sind.
- **Aufräumen**: Einstellungen entfernter Addons, die `.bak`-Kopien von WoW, alte Kampflogs, Screenshots und der
  Cache – mit Größenangaben, mit einem Klick.
- **UI-Profile**: die Oberfläche eines Charakters speichern (Tastenbelegung, Makros, Layout, Addons und ihre
  Profile) und mit zwei Klicks einem anderen Charakter geben.
- Aktualisiert sich selbst: fragt einmal am Tag bei GitHub nach und installiert eine neue Version mit einem Klick.
- Englisch, Russisch, Deutsch, Französisch und Spanisch.

<p>
  <img src="docs/images/panel-de.png" width="270" alt="Reiter „Sicherung“ im Bereich">
  <img src="docs/images/addons-de.png" width="270" alt="Reiter „Addons“ im Bereich">
  <img src="docs/images/menu-de.png" width="210" alt="Menü des Infobereichssymbols">
</p>

## Inhalt

- [Voraussetzungen](#voraussetzungen)
- [Installation](#installation)
- [Das Infobereichssymbol und der Bereich](#das-infobereichssymbol-und-der-bereich)
- [Einstellungen](#einstellungen)
- [Sicherungen: so funktionieren sie](#sicherungen-so-funktionieren-sie)
- [Wiederherstellen](#wiederherstellen)
- [Addon-Updates](#addon-updates)
- [Spiel-Patches](#spiel-patches)
- [Aufräumen](#aufräumen)
- [UI-Profile](#ui-profile)
- [Einstellungen zwischen Charakteren kopieren](#einstellungen-zwischen-charakteren-kopieren)
- [Befehlszeile](#befehlszeile)
- [Dateien und Ordner](#dateien-und-ordner)
- [Datenschutz und Sicherheit](#datenschutz-und-sicherheit)
- [WoW Tools aktualisieren](#wow-tools-aktualisieren)
- [Deinstallation](#deinstallation)
- [FAQ und Einschränkungen](#faq-und-einschränkungen)
- [Aus dem Quellcode bauen](#aus-dem-quellcode-bauen)

## Voraussetzungen

- **Windows 10 oder 11**, 64 Bit. Sonst muss nichts installiert werden: Es ist eine einzelne `.exe` ohne
  Laufzeitabhängigkeiten.
- **World of Warcraft**, installiert über Battle.net. Jede Spielversion wird unterstützt: Retail, Classic (Progression),
  Classic Era / Hardcore / Season of Discovery, Anniversary, PTR, Beta und auch Versionen, die es noch gar nicht gibt –
  alles in einem `_name_`-Ordner neben `_retail_`.
- Nur für Addon-Updates – **eines** dieser Programme, installiert und eingerichtet (angemeldet, Spielordner gewählt):
  - die [CurseForge](https://www.curseforge.com/download/app)-App;
  - [WowUp](https://wowup.io/) oder WowUp-CF (die Variante mit CurseForge-Unterstützung);
  - [Wago App](https://addons.wago.io/app) – nur für WeakAuras und UI-Pakete, siehe [unten](#wago-app).

  Sicherungen, Wiederherstellung und das Kopieren zwischen Charakteren funktionieren auch ohne sie.

## Installation

1. Laden Sie `WoW-Tools-x.y.z.exe` unter [Releases](https://github.com/Gronck2/wow-tools/releases) herunter.
2. Legen Sie die Datei in einen festen Ordner (zum Beispiel `%LOCALAPPDATA%\Programs\WoW Tools\`) – der Autostart
   merkt sich diesen Pfad.
3. Starten Sie sie. Ein Symbol erscheint im Infobereich; der Spielordner wird automatisch gefunden (Registry,
   Battle.net, übliche Pfade).

Windows SmartScreen warnt eventuell vor einem unbekannten Herausgeber, da die exe nicht signiert ist: Wählen Sie
**Weitere Informationen → Trotzdem ausführen**. Zu jedem Release wird die SHA-256-Prüfsumme der Datei angegeben, damit
Sie den Download prüfen können (`Get-FileHash .\WoW-Tools-0.1.0.exe`).

Wird die exe erneut gestartet, während sie bereits läuft, öffnet sich das Einstellungsfenster.

## Das Infobereichssymbol und der Bereich

Das Symbol zeigt den aktuellen Zustand:

| Symbol | Bedeutung |
| --- | --- |
| graues Wappen | das Spiel läuft nicht |
| goldenes Wappen | das Spiel läuft (die Sicherung folgt nach dem Beenden) |
| blauer Punkt | in Arbeit: Sicherung, Wiederherstellung, Addon-Update, Aufräumen oder ein Profil |
| roter Punkt | der letzte Vorgang ist fehlgeschlagen (Mauszeiger auf das Symbol halten oder den Bereich öffnen, um den Grund zu sehen) |

**Linksklick** öffnet den Bereich (im Stil von Windows 11, folgt dem hellen/dunklen Design und der Akzentfarbe):

- Reiter **Sicherung**: wann die letzte Sicherung erstellt wurde und wann gesichert wird, **Jetzt sichern** und die
  neuesten Wiederherstellungspunkte jeder Spielversion mit der Schaltfläche **Zurück** (bestätigt wird direkt in der
  Zeile mit **Wiederherstellen**).
- Reiter **Addons**: wann die Addons zuletzt aktualisiert wurden, **Addons aktualisieren** und die letzten fünf
  Update-Läufe – was aktualisiert wurde oder was schiefging (für den vollständigen Text den Mauszeiger auf die Zeile
  halten).
- Reiter **Aufräumen**: was sich in WTF und im Spielordner aufräumen lässt, mit Größenangaben und je einer
  Schaltfläche – siehe [Aufräumen](#aufräumen).
- Reiter **Profile**: gespeicherte UI-Profile und **Profil speichern** — siehe [UI-Profile](#ui-profile).
- Oben: die Büroklammer, der Sicherungsordner und die Einstellungen; unten: **Mit Windows starten**. Der Bereich
  schließt sich bei einem Klick daneben; mit gedrückter Büroklammer bleibt er offen, bis Sie Esc drücken oder auf das
  Infobereichssymbol klicken.

**Rechtsklick** öffnet das Menü: Bereich, jetzt sichern, Addons aktualisieren, Sicherungsordner, Einstellungen,
Protokoll, Autostart, Beenden.

Benachrichtigungen erscheinen für die Ergebnisse dessen, was Sie selbst gestartet haben, und bei neuen Fehlern;
**Benachrichtigungen** unter **Einstellungen → Allgemein** schaltet sie ab.

## Einstellungen

<p>
  <img src="docs/images/settings-de.png" width="400" alt="Einstellungen, Reiter „Allgemein“">
  <img src="docs/images/settings-addons-de.png" width="400" alt="Einstellungen, Reiter „Addons“">
</p>

Jede Änderung wird sofort gespeichert und ohne Neustart übernommen.

- **Allgemein**: Spielordner (automatisch gefunden, auch manuell festlegbar), mit welchen Spielversionen gearbeitet wird (Sicherungen, Aufräumen, Profile),
  Sprache, Start mit Windows, **Benachrichtigungen**, **Nach Updates suchen** und die aktuelle Version mit **Prüfen** /
  **Installieren** sowie ein Link zu `config.toml` für selten benötigte Optionen.
- **Sicherung**: wann gesichert wird (*Nach dem Spiel*, also beim Beenden, oder *Nach Zeitplan* – an gewählten
  Wochentagen zu einer festen Uhrzeit), wie viele Sicherungen behalten werden, die Schalter **Addons sichern** und
  **Auf Spielende warten**, der Sicherungsordner.
- **Addons**: das Programm für die Updates, **Nicht während des Spielens**, **Automatisch aktualisieren** und wann –
  *Regelmäßig* (alle 30 Min. … 12 Std.) oder *Nach Zeitplan* (Tage und Uhrzeit); darunter jedes Addon, das nicht
  für die aktuelle Spielversion gemacht ist (siehe [Spiel-Patches](#spiel-patches)).
- **Screenshots**: wohin das Aufräumen Screenshots verschiebt und wie es sie sortiert (siehe [Aufräumen](#aufräumen)).

## Sicherungen: so funktionieren sie

- Eine **Sicherung** ist ein Zip-Archiv eines Ordners: `WTF` und `Interface\AddOns` werden getrennt archiviert, und
  zwar für jede Spielversion einzeln. Die Pfade im Archiv sind relativ zum Ordner der Spielversion.
- Gesichert wird **nur, wenn sich etwas geändert hat**: WoW Tools merkt sich einen Fingerabdruck des Ordners
  (Dateipfade, Größen, Änderungszeiten) und überspringt unveränderte Ordner. Addons, die sich selten ändern, kosten
  daher fast nichts.
- **Solange eine Spielversion läuft, werden ihre Sicherungen übersprungen**: WoW schreibt `WTF` und
  Addon-Einstellungen beim Ausloggen oder Beenden, eine Kopie während des Spielens wäre also halb veraltet. Andere
  Versionen werden wie gewohnt gesichert. Im Modus *Nach dem Spiel* wird die Sicherung wenige Sekunden nach dem
  Schließen des Clients erstellt.
- **Nach Zeitplan**: War der PC zur geplanten Zeit aus, wird nach dem Einschalten gesichert; lief das Spiel, direkt
  nach dem Beenden. Eine manuelle Sicherung zählt als geplante.
- **Aufbewahrung**: Die neuesten `keep` Sicherungen (standardmäßig 30) jeder Reihe bleiben erhalten, ältere werden
  gelöscht.
- `*.bak`- und `*.old`-Dateien werden ausgelassen (deshalb passt ein 155 MB großer `WTF`-Ordner meist in ~13 MB). Die
  Liste steht als `exclude_ext` in `config.toml`.

## Wiederherstellen

**Zurück** im Bereich setzt `WTF` und Addons dieser Spielversion auf den damaligen Stand zurück. Die Spielversion muss
geschlossen sein (andere dürfen laufen). Bevor etwas ersetzt wird, wird der aktuelle Stand als Sicherung mit der
Bezeichnung *vor Wiederherstellung* gespeichert – Sie können also jederzeit zurück.

Die Wiederherstellung entpackt das Archiv in einen temporären Ordner neben dem echten und tauscht erst dann die Ordner
aus. Eine unterbrochene Wiederherstellung hinterlässt daher nie ein halb geschriebenes `WTF`.

Die `AddOns`-Sicherungen *vor Update* zeigt der Bereich nicht: `wow-tools list --kind addons` listet sie,
`wow-tools restore <id> --kind addons` macht ein Addon-Update rückgängig und lässt die Einstellungen unverändert.

Vor dem Aufräumen, einem UI-Profil oder dem Kopieren von Einstellungen wird WTF ebenfalls gesichert (*vor Aufräumen*,
*vor Profil*, *vor Kopieren*). Das ist ein Sicherheitsnetz: Der Bereich zeigt davon die *vor Aufräumen* und *vor Profil*; mehrere
solche Aktionen innerhalb von 10 Minuten deckt die eine vor der ersten ab, solange das Spiel WTF dazwischen nicht
ändert. `wow-tools list --kind wtf` zeigt sie,
`wow-tools restore <id> --kind wtf` stellt eine wieder her.

Über die Befehlszeile: `wow-tools restore` (neueste), `wow-tools restore 20261012` (die neueste dieses Tages oder
davor), `--kind wtf` oder `--kind addons` für nur einen Teil, `--flavor _classic_era_` für eine andere Spielversion.

## Addon-Updates

WoW Tools lädt selbst keine Addons herunter (außer [ElvUI](#elvui)). Es startet den Addon-Manager, den Sie bereits
haben, lässt ihn sein übliches Update durchführen und schließt ihn wieder. So bleiben die Daten des Managers korrekt,
und Autoren, die Downloads nur über offizielle Clients erlauben, werden respektiert.

Es wird nur **ein** Programm verwendet: Zwei Manager, die am selben `AddOns`-Ordner arbeiten, überschreiben sich
gegenseitig die Dateien. Haben Sie mehrere, wählen Sie eines unter **Einstellungen → Addons**; ist nur eines
vorhanden, wird es automatisch verwendet.

Bei jedem Lauf, automatisch oder manuell:

1. Ist **Nicht während des Spielens** an (Standard), passiert nichts, solange ein Spielclient läuft; das Update wartet,
   bis das Spiel geschlossen ist.
2. Eine `AddOns`-Sicherung mit der Bezeichnung *vor Update* wird erstellt – `wow-tools restore <id> --kind addons`
   macht das Update rückgängig.
3. Das Programm wird ohne Fenster gestartet, aktualisiert und wird geschlossen. **Ein Programm, das Sie bereits
   geöffnet hatten, bleibt unangetastet** – es wird weder ein zweites Mal gestartet noch geschlossen.
4. Das Ergebnis landet im Update-Verlauf: welche Addons aktualisiert wurden, welche noch warten (zum Beispiel weil sie
   im Programm auf manuelle Updates gestellt sind) oder warum es fehlgeschlagen ist.

Ist das automatische Aktualisieren im Programm aus, wird es nur für diesen Lauf eingeschaltet und danach wieder
ausgeschaltet (auch wenn der Lauf abgebrochen wurde – das wird beim nächsten Mal nachgeholt). Addons, die Sie im
Programm auf manuell gestellt oder ignoriert haben, werden nicht angefasst.

### CurseForge

Wird mit `--minimized` gestartet (in seinen Infobereich, wie bei seinem eigenen Autostart). WoW Tools verfolgt sein
Protokoll, bis die Update-Prüfung abgeschlossen ist und eine Minute lang nichts installiert wurde (höchstens
10 Minuten), und schließt es dann.

### WowUp und WowUp-CF

Wird mit `--hidden --quit` gestartet: eine stille Auto-Update-Runde, danach beendet es sich selbst. WowUp aktualisiert
nur Addons mit aktiviertem Auto-Update; WoW Tools aktiviert es für diesen Lauf bei jedem Addon, das nicht ignoriert
wird.

> **Addons von Wago über WowUp.** Das normale WowUp erhält seine Download-Berechtigung für Wago über die Werbung in
> seinem Fenster. Ohne Fenster gestartet, bekommt es keine, und Wago verweigert den Download. Solche Addons stehen mit
> diesem Grund im Verlauf. Aktualisieren Sie sie, indem Sie WowUp öffnen, hinterlegen Sie einen Wago-Schlüssel in den
> Einstellungen von WowUp, oder nutzen Sie CurseForge / WowUp-CF, wenn die meisten Ihrer Addons von dort stammen.

### Wago App

Die Wago App kann Addons nicht unbeaufsichtigt aktualisieren – nur über ihre Schaltfläche *Update All* –, daher
aktualisiert sie als gewähltes Programm **nur WeakAuras und UI-Pakete**. Ist sie gewählt, erscheinen zwei Schalter:
**WeakAuras** und **UI-Pakete**. Die App wird minimiert gestartet, schreibt ihre Updates nach
`AddOns\WagoAppCompanion` (WeakAuras, werden im Spiel übernommen) und `AddOns\WagoUI_Storage` (UI-Pakete) und wird
geschlossen, sobald sie 15 Sekunden lang nichts mehr getan hat. Ein Teil, den Sie ausgeschaltet haben, bleibt genau
so, wie er war. Sind beide aus, gibt es nichts zu tun, und die App wird nicht gestartet.

### ElvUI

ElvUI gibt es weder bei CurseForge noch bei Wago, daher aktualisiert es kein Addon-Manager. Ist der Schalter **ElvUI**
an (Standard), fragt jeder Update-Lauf zusätzlich die offizielle Website [tukui.org](https://tukui.org/elvui) nach der
aktuellen Version. Jede Spielversion, in der ElvUI installiert und veraltet ist, bekommt das neue Zip: Die Ordner
`ElvUI`, `ElvUI_Libraries` und `ElvUI_Options` werden komplett ersetzt, sodass auch Dateien verschwinden, die eine
ältere Version hinterlassen hat (eine übrig gebliebene `ElvUI_Mainline.toc` würde das Spiel die Dateiliste der alten
Version laden lassen). Eine Installation, in der sich so Versionen mischen, wird auch bei gleicher Versionsnummer neu
installiert. ElvUI-Plugins (`ElvUI_WindTools` und ähnliche) bleiben unberührt, und Spielversionen ohne ElvUI bekommen
nichts. Das funktioniert auch ganz ohne installierten Addon-Manager.

## Spiel-Patches

Einmal pro Minute vergleicht WoW Tools die Version jedes Spielclients (so wie Battle.net sie meldet) mit der zuletzt
gesehenen. Eine neue Version bei geschlossenem Spiel bedeutet, dass Battle.net es gerade gepatcht hat und das Spiel
darauf noch nicht gelaufen ist – der Moment, in dem `WTF` und die Addons noch den Stand von vor dem Patch haben. Dann
macht WoW Tools Folgendes:

1. Es sichert `WTF` und `AddOns` als Sicherungen mit der Bezeichnung *vor Patch* (ein Klick im Bereich holt sie zurück,
   falls das erste Einloggen nach dem Patch etwas zurücksetzt);
2. Es zeigt eine Benachrichtigung mit der neuen Version und den Addons, deren `.toc` nicht dafür gemacht ist (das Spiel
   würde sie als „veraltet“ anzeigen).

Welche Addons nicht für die aktuelle Version gemacht sind, steht immer vollständig unten unter **Einstellungen →
Addons** (die Karte wird gelb, solange es welche gibt) und wird von `wow-tools outdated` ausgegeben. Addons werden nach Haupt- und Nebenversion beurteilt (12.0 → 12.1 zählt, ein
Hotfix 12.1.0 → 12.1.5 nicht); ein Addon ganz ohne Versionsnummer für dieses Spiel (ein reines Classic-Addon in
Retail, eine Bibliothek) wird nicht beurteilt.

## Aufräumen

<p><img src="docs/images/cleanup-de.png" width="360" alt="Bereich, Reiter „Aufräumen“"></p>

Der Reiter **Aufräumen** im Bereich zeigt, was weg kann, mit Anzahl der Dateien und ihrer Größe, für die
Spielversionen, die Sie sichern. Die Liste wird bei jedem Öffnen des Reiters und nach jedem Aufräumen neu eingelesen;
halten Sie den Mauszeiger auf eine Zeile, um zu sehen, worum es geht. Die Listen-Schaltfläche bei *Entfernte Addons*
und *Größte Addon-Einstellungen* öffnet die vollständige Liste mit Größen direkt über dem Reiter
(Mausrad zum Blättern, schließen mit ✕ oder Esc).

| Zeile | Was | Was passiert |
| --- | --- | --- |
| Entfernte Addons | `SavedVariables\*.lua` von Addons, die nicht mehr in `Interface\AddOns` liegen (Blizzards eigene Dateien bleiben) | gelöscht; `WTF` wird vorher als Sicherung *vor Aufräumen* gespeichert |
| .bak- und .old-Kopien | die Kopien, die WoW beim Beenden von jeder Einstellungsdatei anlegt | gelöscht (WoW legt sie neu an; Sicherungen enthalten sie nie) |
| Größte Addon-Einstellungen | Einstellungsdateien ab 5 MB | nur angezeigt: aufgeblähte Dateien verlangsamen das Laden, räumen Sie sie im Addon selbst auf |
| Alte Kampflogs | `Logs\WoWCombatLog*.txt` | gelöscht; Logs, die jünger als eine Woche sind, bleiben, falls ein Upload zu Warcraft Logs noch aussteht |
| Screenshots | `Screenshots\*` | verschoben, standardmäßig nach `Pictures\World of Warcraft\<version>\<YYYY-MM>\` (nichts wird überschrieben); das Ordnersymbol öffnet diesen Ordner |
| Cache und Fehler | `Cache` und `Errors` | gelöscht; das Spiel baut den Cache neu auf – das übliche Mittel gegen Merkwürdigkeiten nach einem Patch |

Der Reiter **Screenshots** der Einstellungen legt fest, wohin sie kommen und wie sie sortiert werden: Ordner
**Nach Tag**, **Nach Monat**, **Nach Jahr** oder **Ohne Ordner** (das Datum stammt aus dem Dateinamen von WoW,
`WoWScrnShot_MMDDYY_HHMMSS`), und **Ein Ordner je Spielversion** an oder aus. Nach Charakter sortieren geht nicht: WoW
schreibt nicht mit, wer einen Screenshot gemacht hat.

Jede Schaltfläche fragt vorher **Sicher?**. Während es läuft, dreht sich ein Symbol
statt der Schaltfläche, das Ergebnis steht danach in der Zeile. Nur der Cache einer laufenden Spielversion wartet, bis
sie geschlossen ist; alles andere wird auch während des Spiels aufgeräumt (diese Dateien hält das Spiel nicht offen). Über die
Befehlszeile:
`wow-tools clean` zeigt dieselbe Liste, `wow-tools clean orphans backups logs screenshots cache` (beliebige davon)
räumt auf.

## UI-Profile

<p><img src="docs/images/profiles-de.png" width="360" alt="Bereich, Reiter „Profile“"></p>

Ein Profil ist alles, was die Oberfläche eines Charakters ausmacht, unter seinem Namen gespeichert: Im Reiter
**Profile** des Bereichs zeigt **Profil speichern → Wählen…** deine Charaktere (zuletzt gespielte zuerst).
**Anwenden…** bei einem Profil fragt, welcher Charakter es bekommt, dann **Anwenden** in der Zeile. Fahre über ein
Profil, um zu sehen, was es enthält; der Papierkorb löscht es. Hat sich am Charakter seit seinem letzten Profil
nichts geändert, wird kein neues gespeichert (die Zeile sagt es).
In der Charakterliste einfach tippen, um nach Name oder Realm zu suchen (Enter wählt den ersten, Esc leert die
Suche). Solange die Spielversion des Profils läuft, ist **Anwenden…** aus: Das Spiel würde die Einstellungen beim
Beenden überschreiben.

Die Charakterlisten zeigen nur Charaktere, die der Account jetzt hat: Ordner gelöschter, umbenannter oder
transferierter Charaktere bleiben in `WTF`, daher vergleicht WoW Tools sie mit der Charakterliste des Spiels
(`character-list-order.txt`, nur GUIDs) und den Namen, die Addons zu diesen GUIDs speichern (BigWigs,
WorldQuestTracker, AllTheThings, RCLootCouncil…). Ein Realm mit einem seitdem nicht gespielten Charakter wird nicht
gefiltert: geraten wird nichts.

Was in ein Profil kommt:

- alle Dateien des Charakterordners `WTF\Account\<ACCOUNT>\<Realm>\<Name>`: Tastenbelegung, Makros, Chat,
  Bearbeitungsmodus-Layout, Klickbelegungen, aktivierte Addons, die eigenen Addon-Einstellungen des Charakters
  (`SavedVariables`);
- bei Addons, die ihre Einstellungen accountweit als Profile speichern (ElvUI, BigWigs, Clique, TomTom und andere
  auf AceDB-Basis): welches Profil der Charakter nutzt und dessen Inhalt.

Beim Anwenden werden die Dateien kopiert und diese Addons des Zielcharakters auf dasselbe Profil umgeschaltet. Auf
demselben Account teilen sich die beiden Charaktere es danach: eine Änderung an ElvUI beim einen zeigt sich auch beim
anderen, wie bei einem gemeinsamen Profil im Spiel. Auf einem anderen Account wird das Profil dort hinzugefügt; gibt
es dort schon ein anderes Profil dieses Namens, bekommt die Kopie den Namen des Quellcharakters, sodass sich auf jenem
Account nichts ändert. Ein Addon, das auf dem Zielaccount nie lief, bleibt unberührt und wird im Ergebnis genannt. Die
Spielversion des Profils muss geschlossen sein; vorher wird WTF *vor Profil* gesichert (siehe
[Wiederherstellen](#wiederherstellen)).

Tastenbelegung und Makros können auf den Servern von Blizzard liegen (siehe Ende des nächsten Abschnitts).

Über die Befehlszeile: `wow-tools profile save "Tarren Mill/Gronckk"`, `wow-tools profile list`,
`wow-tools profile apply 1 --to "Tarren Mill/Groncky"`, `wow-tools profile delete 1`.

## Einstellungen zwischen Charakteren kopieren

Charaktere liegen unter `WTF\Account\<ACCOUNT>\<Realm>\<Name>`. So kopieren Sie die Einrichtung eines Charakters auf
andere:

```
wow-tools chars
wow-tools copy-char --from "Tarren Mill/Groncky" --to "Kazzak/Groncky" --dry-run
wow-tools copy-char --from "Tarren Mill/Groncky" --to "Kazzak/Groncky" --to "Silvermoon/Gronckowski" --parts bindings,macros,savedvars
```

| Teil | Was | Datei |
| --- | --- | --- |
| `bindings` | Tastenbelegung | `bindings-cache.wtf` |
| `macros` | Charaktermakros | `macros-cache.txt` |
| `chat` | Chatfenster und Kanäle | `chat-cache.txt` |
| `config` | Charakter-CVars | `config-cache.wtf` |
| `layout` | Fensterpositionen | `layout-local.txt` |
| `addons` | welche Addons aktiviert sind | `AddOns.txt` |
| `savedvars` | Addon-Einstellungen des Charakters | `SavedVariables\*.lua` |

Ohne `--parts` wird alles kopiert. Bei Namen spielt Groß-/Kleinschreibung keine Rolle (auch bei kyrillischen); gibt es
einen Namen auf mehreren Accounts, schreiben Sie `ACCOUNT/Realm/Name`. `WTF` wird vorher als Sicherung *vor Kopieren*
gespeichert, und die Spielversion muss geschlossen sein.

Tastenbelegung und Makros können auf den Servern von Blizzard gespeichert sein (CVars `synchronizeBindings` /
`synchronizeMacros`); dann kann die Serverkopie die kopierten Dateien beim Einloggen überschreiben.

## Befehlszeile

`wow-tools` ohne Befehl startet das Infobereichssymbol. Alles andere ist auch über die Konsole verfügbar:

| Befehl | Funktion |
| --- | --- |
| `init [--wow-path DIR] [--backup-dir DIR]` | Konfiguration erstellen (der Spielordner wird automatisch gefunden) |
| `status` | gefundene Installation, Spielversionen, Einstellungen, Addon-Programme und ausstehende Updates |
| `backup [--flavor F] [--kind wtf\|addons] [--force]` | Sicherung erstellen (ohne `--force` nur von Geändertem) |
| `list [--flavor F] [--kind K]` | Sicherungen auflisten |
| `restore [ID\|DATE\|latest] [--kind K] [--flavor F] [-y]` | wiederherstellen, vorher den aktuellen Stand sichern |
| `chars [--flavor F]` | Charaktere auflisten |
| `copy-char --from C --to C [--to C…] [--parts …] [--dry-run]` | Einstellungen zwischen Charakteren kopieren |
| `profile save C [--flavor F]` / `profile list` | UI-Profil eines Charakters speichern / Profile anzeigen |
| `profile apply N --to C` / `profile delete N` | Profil N (aus der Liste) auf einen Charakter übertragen / löschen |
| `update-addons` | Addons jetzt über das gewählte Programm aktualisieren |
| `watch [--interval N]` | das, was das Infobereichssymbol tut, in einem Konsolenfenster |
| `schedule logon [--headless]` | Infobereichssymbol bei der Anmeldung starten (`--headless`: `watch` ohne Symbol) |
| `schedule interval [--every MIN]` | Aufgabe in der Aufgabenplanung, die `backup` alle N Minuten ausführt (standardmäßig einmal täglich) |
| `schedule status` / `schedule remove` | Autostart und Aufgabe anzeigen / entfernen |
| `outdated` | Addons, die nicht für die installierte Spielversion gemacht sind |
| `clean [orphans\|backups\|logs\|screenshots\|cache …]` | anzeigen, was sich aufräumen lässt; mit Angaben aufräumen |
| `self-update [--check]` | bei GitHub nach einer neuen Version von WoW Tools suchen und sie installieren |

Allgemeine Optionen: `--config FILE` (andere Konfiguration), `--log-to-file`, `-v`. `wow-tools <command> --help`
erklärt den Rest in der gewählten Sprache.

## Dateien und Ordner

| Pfad | Was |
| --- | --- |
| `%APPDATA%\WoW Tools\config\config.toml` | Einstellungen |
| `%LOCALAPPDATA%\WoW Tools\data\backups\<version>\wtf\*.zip`, `...\addons\*.zip` | Sicherungen (Ordner änderbar) |
| `%LOCALAPPDATA%\WoW Tools\data\backups\profiles\<version>\*.zip` | UI-Profile (im Sicherungsordner) |
| `%LOCALAPPDATA%\WoW Tools\data\wow-tools.log` | Protokoll des Infobereichssymbols |
| `%LOCALAPPDATA%\WoW Tools\data\addon-updates.json` | Update-Verlauf (die letzten 20) |
| `%LOCALAPPDATA%\WoW Tools\data\self-update.json` | wann WoW Tools zuletzt nach einer neuen Version gesucht hat |
| `%LOCALAPPDATA%\WoW Tools\data\game-versions.json` | die zuletzt gesehenen Spielversionen, um einen Patch zu erkennen |

Sicherungen heißen `YYYYmmdd-HHMMSS[-label].zip`; es sind gewöhnliche Zip-Dateien, die sich auch von Hand öffnen
lassen.

`config.toml` enthält außerdem, was das Fenster nicht zeigt: `exclude_ext` (ausgelassene Dateiendungen, standardmäßig
`["bak", "old"]`) und `language` (`ru`, `en`, `de`, `fr`, `es`; fehlt der Eintrag, gilt die Anzeigesprache von
Windows).

## Datenschutz und Sicherheit

- WoW Tools baut selbst Verbindungen zu zwei Stellen auf. Zu **GitHub**, um nach einer neuen Version von WoW Tools
  zu suchen (siehe [WoW Tools aktualisieren](#wow-tools-aktualisieren)) – einmal am Tag, und das lässt sich
  ausschalten. Und zu **tukui.org** während eines Addon-Updates, wenn ElvUI installiert und sein Schalter an ist, für
  [ElvUI](#elvui). Beides sind einfache Abfragen der neuesten Version, wie das Öffnen der Download-Seite im Browser;
  über Sie, Ihr Spiel oder Ihre Addons wird nichts gesendet.
- Addons lädt nur das von Ihnen gewählte Addon-Programm herunter – außer ElvUI, das WoW Tools von seiner offiziellen
  Website holt.
- Das Spiel selbst wird nie angefasst: kein Lesen des Spielspeichers, keine Automatisierung im Spiel.
- In den Spielordner wird nur geschrieben, wenn Sie wiederherstellen, Charaktereinstellungen kopieren, aufräumen
  oder ein Addon-Programm Addons aktualisiert – und vor allem, was Einstellungen oder Addons betrifft, wird eine
  Sicherung erstellt.

## WoW Tools aktualisieren

WoW Tools prüft einmal am Tag die [Releases](https://github.com/Gronck2/wow-tools/releases) dieses Repositorys (und
wenn Sie unter **Einstellungen → Allgemein** auf **Prüfen** klicken). Gibt es eine neuere Version:

- erscheint einmalig eine Benachrichtigung;
- zeigt der Bereich neben den Symbolen für Ordner und Einstellungen eine Schaltfläche mit der neuen Versionsnummer;
- bekommt das Menü den Eintrag **Auf x.y.z aktualisieren**, und die Einstellungen zeigen **Installieren**.

Ein Klick lädt die neue exe herunter, prüft sie gegen die SHA-256-Prüfsumme, die GitHub für die Release-Datei
veröffentlicht (eine Datei, die nicht passt, wird nie installiert), ersetzt damit die aktuelle und startet das
Infobereichssymbol neu. Die alte exe bleibt bis zum nächsten Start als `<name>.exe.old` erhalten und wird dann
entfernt. Läuft gerade eine Sicherung oder ein Update, wartet der Neustart, bis es fertig ist. Ihre Einstellungen,
Sicherungen und der Autostart bleiben unverändert.

Über die Befehlszeile: `wow-tools self-update --check` (nur prüfen) oder `wow-tools self-update` (prüfen und
installieren; das Infobereichssymbol danach neu starten). Um die Suche abzuschalten, deaktivieren Sie
**Nach Updates suchen** unter **Einstellungen → Allgemein** (`check_updates = false` in `config.toml`).

## Deinstallation

1. Schalten Sie **Mit Windows starten** aus (oder führen Sie `wow-tools schedule remove` aus, das auch eine geplante
   Aufgabe entfernt).
2. Wählen Sie **Beenden** im Menü des Infobereichssymbols.
3. Löschen Sie die exe und, falls Sie Einstellungen und Sicherungen nicht mehr brauchen, `%APPDATA%\WoW Tools` und
   `%LOCALAPPDATA%\WoW Tools`.

## FAQ und Einschränkungen

- **Die Wiederherstellung meldet, dass das Spiel läuft.** Schließen Sie diese Spielversion (der Battle.net-Launcher
  darf offen bleiben).
- **Kopierte Tastenbelegungen sind nach dem Einloggen verschwunden.** Sie werden mit dem Server synchronisiert; siehe
  [oben](#einstellungen-zwischen-charakteren-kopieren).
- **Das Update meldet „Keine Updates.“, das Programm zeigt aber Updates an.** Diese Addons sind im Programm
  auf manuell gestellt oder ignoriert oder stammen (bei WowUp) von Wago – den Grund zeigt die Zeile im Verlauf.
- **Lassen sich mehrere Addon-Programme gleichzeitig nutzen?** Nein, mit Absicht: Sie machen sich gegenseitig die
  Addons kaputt.
- **Wo wird der Spielordner gesucht?** In der Registry, in der Installationsliste von Battle.net, dann unter den
  üblichen Pfaden (`C:\Program Files (x86)\World of Warcraft`, `X:\Games\World of Warcraft`, …). Spielversionen, die
  Battle.net in andere Ordner installiert hat, werden ebenfalls gefunden. Andernfalls legen Sie ihn unter
  **Einstellungen → Allgemein** fest.

## Aus dem Quellcode bauen

```
cargo build --release
```

Rust stable, Windows. Die CI (GitHub Actions) prüft bei jedem Push Formatierung, Clippy, Tests und den Build; ein Tag
`vX.Y.Z`, der zur Version in `Cargo.toml` passt, baut die exe und legt einen Release-Entwurf an. Die Texte liegen in
`locales/<language>.toml`.
