# WoW Tools

[English](README.md) · [Русский](README.ru.md) · [Deutsch](README.de.md) · **Français** · [Español](README.es.md)

Une petite application Windows pour World of Warcraft, dans la zone de notification, qui met vos réglages et vos addons
à l'abri :

- **Sauvegardes** de `WTF` (raccourcis clavier, macros, discussion, CVars, réglages des addons) et de `Interface\AddOns`
  — après chaque session de jeu ou selon un planning, pour toutes les versions du jeu installées.
- **Restauration en un clic** vers n'importe quel point antérieur, l'état actuel étant d'abord enregistré : une
  restauration peut toujours être annulée.
- **Copie des réglages entre personnages** : raccourcis, macros, fenêtres de discussion, disposition des cadres, addons
  activés, réglages des addons.
- **Mise à jour des addons** via le gestionnaire que vous utilisez déjà — CurseForge, WowUp, WowUp-CF ou Wago App —
  lancé sans fenêtre, selon un planning, jamais pendant que vous jouez. ElvUI, qu'aucun d'eux ne propose, directement
  depuis son site officiel.
- **Patchs du jeu** : quand Battle.net met le jeu à jour, les réglages et les addons sont sauvegardés avant le premier
  lancement sur la nouvelle version, et vous voyez quels addons ne sont pas encore faits pour elle.
- **Nettoyage** : réglages d'addons supprimés, copies `.bak` de WoW, vieux journaux de combat, captures d'écran et
  cache — avec leur taille, en un clic.
- **Profils d'interface** : enregistrez l'interface d'un personnage (raccourcis, macros, disposition, addons et
  leurs profils) et appliquez-la à un autre personnage en deux clics.
- Mise à jour automatique de l'application : vérification sur GitHub une fois par jour et installation d'une nouvelle
  version en un clic.
- Anglais, russe, allemand, français et espagnol.

<p>
  <img src="docs/images/panel-fr.png" width="270" alt="Onglet Sauvegarde du panneau">
  <img src="docs/images/addons-fr.png" width="270" alt="Onglet Addons du panneau">
  <img src="docs/images/menu-fr.png" width="210" alt="Menu de l'icône de notification">
</p>

## Sommaire

- [Prérequis](#prérequis)
- [Installation](#installation)
- [L'icône de notification et le panneau](#licône-de-notification-et-le-panneau)
- [Paramètres](#paramètres)
- [Fonctionnement des sauvegardes](#fonctionnement-des-sauvegardes)
- [Restauration](#restauration)
- [Mise à jour des addons](#mise-à-jour-des-addons)
- [Patchs du jeu](#patchs-du-jeu)
- [Nettoyage](#nettoyage)
- [Profils d'interface](#profils-dinterface)
- [Copier les réglages entre personnages](#copier-les-réglages-entre-personnages)
- [Ligne de commande](#ligne-de-commande)
- [Fichiers et dossiers](#fichiers-et-dossiers)
- [Confidentialité et sécurité](#confidentialité-et-sécurité)
- [Mettre à jour WoW Tools](#mettre-à-jour-wow-tools)
- [Désinstallation](#désinstallation)
- [FAQ et limites](#faq-et-limites)
- [Compiler depuis les sources](#compiler-depuis-les-sources)

## Prérequis

- **Windows 10 ou 11**, 64 bits. Rien d'autre à installer : c'est un seul `.exe`, sans dépendance d'exécution.
- **World of Warcraft** installé via Battle.net. Toutes les versions du jeu sont prises en charge : Retail, Classic
  (progression), Classic Era / Hardcore / Season of Discovery, Anniversary, PTR, bêta, et même les versions qui
  n'existent pas encore — tout dossier `_nom_` placé à côté de `_retail_`.
- Pour la mise à jour des addons uniquement — **un** de ces programmes, installé et configuré (compte connecté,
  dossier du jeu choisi) :
  - l'application [CurseForge](https://www.curseforge.com/download/app) ;
  - [WowUp](https://wowup.io/) ou WowUp-CF (la version avec CurseForge) ;
  - [Wago App](https://addons.wago.io/app) — uniquement pour WeakAuras et les packs d'interface, voir [plus bas](#wago-app).

  Les sauvegardes, la restauration et la copie entre personnages fonctionnent sans aucun d'eux.

## Installation

1. Téléchargez `WoW-Tools-x.y.z.exe` depuis les [Releases](https://github.com/Gronck2/wow-tools/releases).
2. Placez-le dans un dossier définitif (par exemple `%LOCALAPPDATA%\Programs\WoW Tools\`) — le démarrage automatique
   retient ce chemin.
3. Lancez-le. Une icône apparaît dans la zone de notification ; le dossier du jeu est trouvé automatiquement (registre,
   Battle.net, chemins courants).

Windows SmartScreen peut signaler un éditeur inconnu, car l'exe n'est pas signé : choisissez
**Informations complémentaires → Exécuter quand même**. Chaque release indique le SHA-256 du fichier pour vérifier ce
que vous avez téléchargé (`Get-FileHash .\WoW-Tools-0.1.0.exe`).

Relancer l'exe alors qu'il tourne déjà ouvre la fenêtre des paramètres.

## L'icône de notification et le panneau

L'icône indique l'état actuel :

| Icône | Signification |
| --- | --- |
| emblème gris | le jeu n'est pas lancé |
| emblème doré | le jeu est lancé (la sauvegarde se fera après sa fermeture) |
| point bleu | travail en cours : sauvegarde, restauration, mise à jour des addons, nettoyage ou profil |
| point rouge | la dernière opération a échoué (survolez l'icône ou ouvrez le panneau pour savoir pourquoi) |

Le **clic gauche** ouvre le panneau (style Windows 11, suit le thème clair/sombre et la couleur d'accentuation) :

- Onglet **Sauvegarde** : l'heure de la dernière sauvegarde et quand les sauvegardes ont lieu, **Sauvegarder**, et les
  derniers points de restauration de chaque version du jeu avec un bouton **Restaurer** (confirmation directement dans
  la ligne).
- Onglet **Addons** : la date de la dernière mise à jour des addons, **Mettre à jour les addons**, et les cinq
  dernières mises à jour — ce qui a été mis à jour ou ce qui a échoué (survolez une ligne pour voir le texte complet).
- Onglet **Nettoyage** : ce qui peut être rangé dans WTF et le dossier du jeu, avec les tailles et un bouton pour
  chaque élément — voir [Nettoyage](#nettoyage).
- Onglet **Profils** : les profils d'interface enregistrés et **Enregistrer un profil** — voir
  [Profils d'interface](#profils-dinterface).
- En haut : le trombone, le dossier des sauvegardes et les paramètres ; en bas : **Lancer avec Windows**. Le panneau
  se ferme quand vous cliquez ailleurs ; trombone enfoncé, il reste ouvert jusqu'à Échap ou un clic sur l'icône de
  notification.

Le **clic droit** ouvre le menu : le panneau, sauvegarder, mettre à jour les addons, dossier des sauvegardes,
paramètres, journal, démarrage automatique, quitter.

Des notifications s'affichent pour le résultat de ce que vous avez lancé vous-même et pour les nouvelles erreurs ;
**Notifications** dans **Paramètres → Général** les désactive.

## Paramètres

<p>
  <img src="docs/images/settings-fr.png" width="400" alt="Paramètres, onglet Général">
  <img src="docs/images/settings-addons-fr.png" width="400" alt="Paramètres, onglet Addons">
</p>

Chaque modification est enregistrée aussitôt et appliquée sans redémarrage.

- **Général** : dossier du jeu (trouvé automatiquement, modifiable à la main), versions du jeu avec lesquelles travailler (sauvegardes, nettoyage, profils), langue,
  lancement avec Windows, **Notifications**, **Rechercher les mises à jour** et la version actuelle avec **Vérifier** /
  **Installer**, et un lien vers `config.toml` pour les options rarement utiles.
- **Sauvegarde** : quand sauvegarder (*Après le jeu*, à sa fermeture, ou *Planifié* — les jours choisis à une heure
  donnée), nombre de sauvegardes à conserver, les interrupteurs **Sauvegarder les addons** et **Attendre la fermeture du jeu**,
  dossier des sauvegardes.
- **Addons** : le programme de mise à jour, **Pas de mise à jour en jeu**, **Mise à jour automatique**, et quand —
  *Périodique* (toutes les 30 min … 12 h) ou *Planifié* (jours et heure) ; en dessous, tous les addons qui ne sont pas
  faits pour la version actuelle du jeu (voir [Patchs du jeu](#patchs-du-jeu)).
- **Captures** : où le nettoyage déplace les captures d'écran et comment il les classe (voir [Nettoyage](#nettoyage)).

## Fonctionnement des sauvegardes

- Une **sauvegarde** est une archive zip d'un dossier : `WTF` et `Interface\AddOns` sont archivés séparément, et
  chaque version du jeu à part. À l'intérieur, les chemins sont relatifs au dossier de la version du jeu.
- Une sauvegarde n'est créée **que si quelque chose a changé** : WoW Tools retient une empreinte du dossier (chemins,
  tailles et dates de modification des fichiers) et ignore les dossiers inchangés. Les addons, qui changent rarement,
  ne coûtent presque rien.
- **Tant qu'une version du jeu est lancée, ses sauvegardes sont ignorées** : WoW écrit `WTF` et les réglages des addons
  à la déconnexion ou en quittant, une copie faite en cours de partie serait donc à moitié périmée. Les autres versions
  sont sauvegardées normalement. En mode *Après le jeu*, la sauvegarde a lieu quelques secondes après la fermeture du
  client.
- **Planifié** : si l'ordinateur était éteint à l'heure prévue, la sauvegarde se fait à son allumage ; si le jeu était
  lancé, juste après sa fermeture. Une sauvegarde manuelle compte comme une sauvegarde planifiée.
- **Conservation** : les `keep` dernières sauvegardes (30 par défaut) de chaque série sont conservées, les plus
  anciennes sont supprimées.
- Les fichiers `*.bak` et `*.old` sont exclus (c'est pourquoi un `WTF` de 155 Mo tient généralement dans ~13 Mo). La
  liste se règle avec `exclude_ext` dans `config.toml`.

## Restauration

**Restaurer** dans le panneau ramène `WTF` et les addons de cette version du jeu à ce moment-là. La version du jeu
concernée doit être fermée (les autres peuvent tourner). Avant tout remplacement, l'état actuel est enregistré dans une
sauvegarde étiquetée *avant restauration* : vous pouvez toujours revenir en arrière.

La restauration extrait l'archive dans un dossier temporaire à côté du vrai, puis échange les dossiers : une
restauration interrompue ne laisse jamais un `WTF` à moitié écrit.

Les sauvegardes de `AddOns` *avant mise à jour* ne sont pas listées dans le panneau : `wow-tools list --kind addons`
les affiche, `wow-tools restore <id> --kind addons` annule une mise à jour des addons sans toucher aux réglages.

Avant un nettoyage, un profil d'interface ou une copie de réglages, WTF est aussi sauvegardé (*avant nettoyage*,
*avant profil*, *avant copie*). C'est un filet de sécurité : le panneau en liste celles *avant nettoyage* et *avant profil* ; plusieurs
de ces actions en 10 minutes partagent celle faite avant la première, sauf si le jeu a modifié WTF entre-temps. `wow-tools list --kind wtf` les affiche,
`wow-tools restore <id> --kind wtf` en restaure une.

En ligne de commande : `wow-tools restore` (la plus récente), `wow-tools restore 20261012` (la plus récente de ce jour
ou d'avant), `--kind wtf` ou `--kind addons` pour une seule partie, `--flavor _classic_era_` pour une autre version du
jeu.

## Mise à jour des addons

WoW Tools ne télécharge pas les addons lui-même (sauf [ElvUI](#elvui)). Il lance le gestionnaire d'addons que vous
avez déjà, le laisse faire sa mise à jour habituelle, puis le ferme. Les données du gestionnaire restent ainsi
correctes, et les auteurs qui n'autorisent le téléchargement que via les clients officiels sont respectés.

Un **seul** programme est utilisé : deux gestionnaires qui travaillent sur le même dossier `AddOns` écrasent les
fichiers l'un de l'autre. Si vous en avez plusieurs, choisissez-en un dans **Paramètres → Addons** ; s'il n'y en a
qu'un, il est utilisé automatiquement.

À chaque exécution, automatique ou manuelle :

1. Avec **Pas de mise à jour en jeu** activé (par défaut), rien ne se passe tant qu'un client du jeu tourne ; la mise
   à jour attend la fermeture du jeu.
2. Une sauvegarde de `AddOns` étiquetée *avant mise à jour* est créée — `wow-tools restore <id> --kind addons`
   annule la mise à jour.
3. Le programme est lancé sans fenêtre, fait la mise à jour, puis est fermé. **Un programme que vous aviez déjà ouvert
   n'est pas touché** — il n'est ni relancé, ni fermé.
4. Le résultat va dans l'historique des mises à jour : les addons mis à jour, ceux qui attendent encore (par exemple
   réglés en mise à jour manuelle dans le programme), ou la raison de l'échec.

Si la mise à jour auto est désactivée dans le programme, elle est activée juste pour cette exécution puis désactivée
de nouveau (même si l'exécution a été interrompue — c'est rétabli la fois suivante). Les addons que vous avez réglés
en manuel ou ignorés dans le programme ne sont pas touchés.

### CurseForge

Lancé avec `--minimized` (dans sa zone de notification, comme le fait son propre démarrage automatique). WoW Tools suit
son journal jusqu'à ce que la recherche de mises à jour soit terminée et que rien n'ait été installé depuis une minute
(10 minutes au maximum), puis le ferme.

### WowUp et WowUp-CF

Lancé avec `--hidden --quit` : une passe de mise à jour auto silencieuse, puis il se ferme tout seul. WowUp ne met à
jour que les addons dont la mise à jour auto est activée ; WoW Tools l'active, pour cette exécution, pour tous les
addons non ignorés.

> **Addons Wago via WowUp.** WowUp standard obtient son autorisation de téléchargement Wago grâce à la publicité
> affichée dans sa fenêtre. Lancé sans fenêtre, il n'en obtient pas, et Wago refuse le téléchargement. Ces addons
> apparaissent dans l'historique avec cette raison. Mettez-les à jour en ouvrant WowUp, indiquez une clé Wago dans les
> paramètres de WowUp, ou utilisez CurseForge / WowUp-CF si la plupart de vos addons en viennent.

### Wago App

Wago App ne sait pas mettre à jour les addons sans intervention — seulement via son bouton *Update All* — donc, choisi
comme programme, il ne met à jour **que WeakAuras et les packs d'interface**. Deux interrupteurs apparaissent quand il
est choisi : **WeakAuras** et **packs d'interface**. L'application est lancée réduite, écrit ses mises à jour dans
`AddOns\WagoAppCompanion` (WeakAuras, à appliquer en jeu) et `AddOns\WagoUI_Storage` (packs d'interface), puis est
fermée après 15 secondes d'inactivité. Une partie que vous avez désactivée reste exactement telle qu'elle était. Si les
deux sont désactivées, il n'y a rien à faire et l'application n'est pas lancée.

### ElvUI

ElvUI n'est ni sur CurseForge ni sur Wago, donc aucun gestionnaire d'addons ne le met à jour. Avec l'interrupteur
**ElvUI** activé (par défaut), chaque mise à jour demande aussi la version actuelle au site officiel
[tukui.org](https://tukui.org/elvui). Chaque version du jeu où ElvUI est installé et en retard reçoit le nouveau zip :
les dossiers `ElvUI`, `ElvUI_Libraries` et `ElvUI_Options` sont remplacés en entier, donc les fichiers laissés par une
ancienne version disparaissent aussi (un `ElvUI_Mainline.toc` oublié ferait charger au jeu la liste de fichiers de
l'ancienne version). Une installation qui mélange ainsi des versions est réinstallée même si le numéro de version est
le même. Les plugins d'ElvUI (`ElvUI_WindTools` et autres) ne sont pas touchés, et les versions du jeu sans ElvUI ne
reçoivent rien. Cela fonctionne aussi sans aucun gestionnaire d'addons installé.

## Patchs du jeu

Une fois par minute, WoW Tools compare la version de chaque client du jeu (telle que Battle.net l'indique) avec la
dernière qu'il a vue. Une nouvelle version alors que ce jeu est fermé signifie que Battle.net vient de le patcher et que
le jeu n'a pas encore tourné dessus — le moment où `WTF` et les addons sont encore dans leur état d'avant le patch.
WoW Tools alors :

1. enregistre `WTF` et `AddOns` dans des sauvegardes étiquetées *avant patch* (un clic dans le panneau les ramène si
   la première connexion après le patch réinitialise quelque chose) ;
2. affiche une notification avec la nouvelle version et les addons dont le `.toc` n'a pas été fait pour elle (le jeu
   les afficherait comme « périmés »).

Les addons qui ne sont pas faits pour la version actuelle sont toujours listés en entier en bas de **Paramètres → Addons**
(la carte devient jaune tant qu'il y en a), et affichés par `wow-tools outdated`. Les addons sont jugés sur la version majeure et mineure (12.0 → 12.1 compte, un
correctif 12.1.0 → 12.1.5 non) ; un addon sans aucun numéro de version pour ce jeu (un addon uniquement Classic sur
Retail, une bibliothèque) n'est pas jugé.

## Nettoyage

<p><img src="docs/images/cleanup-fr.png" width="360" alt="Panneau, onglet Nettoyage"></p>

L'onglet **Nettoyage** du panneau montre ce qui peut partir, avec le nombre de fichiers et leur taille, pour les
versions du jeu que vous sauvegardez. La liste est relue à chaque ouverture de l'onglet et après chaque nettoyage ;
survolez une ligne pour savoir de quoi il s'agit. Le bouton liste de *Addons supprimés* et de *Plus gros réglages*
ouvre la liste complète, avec les tailles, par-dessus l'onglet (molette pour défiler, ✕ ou Échap pour
fermer).

| Ligne | Contenu | Ce qui se passe |
| --- | --- | --- |
| Addons supprimés | `SavedVariables\*.lua` des addons qui ne sont plus dans `Interface\AddOns` (les fichiers de Blizzard restent) | supprimés ; `WTF` est d'abord enregistré dans une sauvegarde *avant nettoyage* |
| Copies .bak et .old | les copies que WoW fait de chaque fichier de réglages en quittant | supprimées (WoW les recrée ; les sauvegardes ne les incluent jamais) |
| Plus gros réglages | fichiers de réglages de 5 Mo et plus | seulement affichés : les fichiers trop gros ralentissent le chargement, nettoyez-les dans l'addon lui-même |
| Vieux journaux de combat | `Logs\WoWCombatLog*.txt` | supprimés ; les journaux de moins d'une semaine restent, au cas où un envoi vers Warcraft Logs serait en attente |
| Captures d'écran | `Screenshots\*` | déplacées, par défaut vers `Pictures\World of Warcraft\<version>\<YYYY-MM>\` (« Images » dans l'Explorateur ; rien n'est écrasé) ; l'icône de dossier ouvre ce dossier |
| Cache et erreurs | `Cache` et `Errors` | supprimés ; le jeu reconstruit le cache — le remède habituel aux bizarreries après un patch |

L'onglet **Captures** des paramètres règle où elles vont et comment elles sont classées :
dossiers **Par jour**, **Par mois**, **Par année** ou **Sans dossiers** (la date vient du nom de fichier de WoW,
`WoWScrnShot_MMDDYY_HHMMSS`), et **Un dossier par version du jeu** activé ou non. Classer par personnage est
impossible : WoW n'écrit pas qui a pris la capture.

Chaque bouton demande d'abord **Sûr ?**. Pendant le travail, une roue tourne à la place
du bouton, puis le résultat s'affiche dans la ligne. Seul le cache d'une version du jeu lancée attend sa fermeture ; le
reste se range même en jouant (le jeu ne garde pas ces fichiers ouverts). En ligne de commande :
`wow-tools clean` affiche la même liste, `wow-tools clean orphans backups logs screenshots cache` (au choix) fait le
ménage.

## Profils d'interface

<p><img src="docs/images/profiles-fr.png" width="360" alt="Panneau, onglet Profils"></p>

Un profil, c'est tout ce qui fait l'interface d'un personnage, enregistré sous son nom : dans l'onglet **Profils** du
panneau, **Enregistrer un profil → Choisir…** liste vos personnages (les plus récemment joués en premier).
**Appliquer…** sur un profil demande à quel personnage l'appliquer, puis **Appliquer** dans la ligne. Survolez un
profil pour voir ce qu'il contient ; la corbeille le supprime. Si rien n'a changé pour le personnage depuis son
dernier profil, aucun nouveau n'est enregistré (la ligne l'indique).
Dans la liste des personnages, tapez simplement pour chercher un nom ou un royaume (Entrée choisit le premier, Échap
efface la recherche). Tant que la version du jeu du profil tourne, **Appliquer…** est désactivé : le jeu réécrirait
les réglages en quittant.

Les listes de personnages ne montrent que ceux que le compte a maintenant : les dossiers des personnages supprimés,
renommés ou transférés restent dans `WTF`, donc WoW Tools les compare à la liste des personnages du jeu
(`character-list-order.txt`, GUID seulement) et aux noms que les addons gardent pour ces GUID (BigWigs,
WorldQuestTracker, AllTheThings, RCLootCouncil…). Un royaume avec un personnage pas joué depuis n'est pas filtré :
rien n'est caché au hasard.

Ce que contient un profil :

- tous les fichiers du dossier du personnage `WTF\Account\<ACCOUNT>\<Realm>\<Name>` : raccourcis, macros, discussion,
  disposition du mode édition, raccourcis de clic, addons activés, réglages d'addons propres au personnage
  (`SavedVariables`) ;
- pour les addons qui gardent leurs réglages au niveau du compte sous forme de profils (ElvUI, BigWigs, Clique, TomTom
  et d'autres basés sur AceDB) : le profil qu'utilise le personnage et son contenu.

L'application copie les fichiers et bascule ces addons du personnage cible sur le même profil. Sur le même compte,
les deux personnages le partagent ensuite : un changement d'ElvUI sur l'un se voit sur l'autre, comme avec un profil
partagé dans le jeu. Sur un autre compte, le profil y est ajouté ; si ce compte a déjà un autre profil de ce nom, la
copie prend le nom du personnage source, et rien ne change sur cet autre compte. Un addon qui n'a jamais tourné sur le
compte cible n'est pas touché et il est cité dans le résultat. La version du jeu du profil doit être fermée ; une
sauvegarde de WTF *avant profil* est faite d'abord (voir [Restauration](#restauration)).

Les raccourcis et les macros peuvent être stockés sur les serveurs de Blizzard (voir la fin de la section suivante).

En ligne de commande : `wow-tools profile save "Tarren Mill/Gronckk"`, `wow-tools profile list`,
`wow-tools profile apply 1 --to "Tarren Mill/Groncky"`, `wow-tools profile delete 1`.

## Copier les réglages entre personnages

Les personnages se trouvent dans `WTF\Account\<ACCOUNT>\<Realm>\<Name>`. Pour copier la configuration d'un personnage
vers d'autres :

```
wow-tools chars
wow-tools copy-char --from "Tarren Mill/Groncky" --to "Kazzak/Groncky" --dry-run
wow-tools copy-char --from "Tarren Mill/Groncky" --to "Kazzak/Groncky" --to "Silvermoon/Gronckowski" --parts bindings,macros,savedvars
```

| Partie | Contenu | Fichier |
| --- | --- | --- |
| `bindings` | raccourcis clavier | `bindings-cache.wtf` |
| `macros` | macros du personnage | `macros-cache.txt` |
| `chat` | fenêtres et canaux de discussion | `chat-cache.txt` |
| `config` | CVars du personnage | `config-cache.wtf` |
| `layout` | position des cadres | `layout-local.txt` |
| `addons` | addons activés | `AddOns.txt` |
| `savedvars` | réglages des addons du personnage | `SavedVariables\*.lua` |

Sans `--parts`, tout est copié. Les noms ne tiennent pas compte de la casse (cyrillique compris) ; si un nom existe sur
plusieurs comptes, écrivez `ACCOUNT/Realm/Name`. `WTF` est d'abord enregistré dans une sauvegarde *avant copie*, et la
version du jeu doit être fermée.

Les raccourcis clavier et les macros peuvent être stockés sur les serveurs de Blizzard (CVars `synchronizeBindings` /
`synchronizeMacros`) ; dans ce cas, la copie du serveur peut remplacer les fichiers copiés à la connexion.

## Ligne de commande

`wow-tools` sans commande lance l'icône de notification. Tout le reste est aussi disponible depuis une console :

| Commande | Ce qu'elle fait |
| --- | --- |
| `init [--wow-path DIR] [--backup-dir DIR]` | crée la configuration (le dossier du jeu est trouvé automatiquement) |
| `status` | l'installation trouvée, les versions du jeu, les paramètres, les programmes d'addons et les mises à jour en attente |
| `backup [--flavor F] [--kind wtf\|addons] [--force]` | crée une sauvegarde (seulement de ce qui a changé, sauf avec `--force`) |
| `list [--flavor F] [--kind K]` | liste les sauvegardes |
| `restore [ID\|DATE\|latest] [--kind K] [--flavor F] [-y]` | restaure, en enregistrant d'abord l'état actuel |
| `chars [--flavor F]` | liste les personnages |
| `copy-char --from C --to C [--to C…] [--parts …] [--dry-run]` | copie les réglages entre personnages |
| `profile save C [--flavor F]` / `profile list` | enregistre le profil d'interface d'un personnage / liste les profils |
| `profile apply N --to C` / `profile delete N` | applique le profil N (de la liste) à un personnage / le supprime |
| `update-addons` | met à jour les addons maintenant via le programme choisi |
| `watch [--interval N]` | ce que fait l'icône de notification, dans une fenêtre de console |
| `schedule logon [--headless]` | lance l'icône de notification à la connexion (`--headless` : `watch` sans icône) |
| `schedule interval [--every MIN]` | une tâche du Planificateur de tâches qui lance `backup` toutes les N minutes (une fois par jour par défaut) |
| `schedule status` / `schedule remove` | affiche / supprime le démarrage automatique et la tâche |
| `outdated` | les addons qui ne sont pas faits pour la version installée du jeu |
| `clean [orphans\|backups\|logs\|screenshots\|cache …]` | affiche ce qui peut être nettoyé ; avec des éléments, les nettoie |
| `self-update [--check]` | cherche une nouvelle version de WoW Tools sur GitHub et l'installe |

Options communes : `--config FILE` (une autre configuration), `--log-to-file`, `-v`. `wow-tools <command> --help`
explique le reste dans la langue choisie.

## Fichiers et dossiers

| Chemin | Contenu |
| --- | --- |
| `%APPDATA%\WoW Tools\config\config.toml` | paramètres |
| `%LOCALAPPDATA%\WoW Tools\data\backups\<version>\wtf\*.zip`, `...\addons\*.zip` | sauvegardes (dossier modifiable) |
| `%LOCALAPPDATA%\WoW Tools\data\backups\profiles\<version>\*.zip` | profils d'interface (dans le dossier des sauvegardes) |
| `%LOCALAPPDATA%\WoW Tools\data\wow-tools.log` | journal de l'icône de notification |
| `%LOCALAPPDATA%\WoW Tools\data\addon-updates.json` | historique des mises à jour (20 dernières) |
| `%LOCALAPPDATA%\WoW Tools\data\self-update.json` | date de la dernière recherche d'une nouvelle version de WoW Tools |
| `%LOCALAPPDATA%\WoW Tools\data\game-versions.json` | les dernières versions du jeu vues, pour repérer un patch |

Les sauvegardes sont nommées `YYYYmmdd-HHMMSS[-label].zip` ; ce sont des fichiers zip ordinaires, que l'on peut ouvrir
à la main.

`config.toml` contient aussi ce que la fenêtre n'affiche pas : `exclude_ext` (extensions exclues, `["bak", "old"]` par
défaut) et `language` (`ru`, `en`, `de`, `fr`, `es` ; absent = la langue d'affichage de Windows).

## Confidentialité et sécurité

- WoW Tools se connecte lui-même à deux endroits. À **GitHub**, pour chercher une nouvelle version de WoW Tools (voir
  [Mettre à jour WoW Tools](#mettre-à-jour-wow-tools)) — une fois par jour, et cela peut être désactivé. Et à
  **tukui.org**, pendant une mise à jour des addons quand ElvUI est installé et que son interrupteur est activé, pour
  [ElvUI](#elvui). Ce sont de simples requêtes pour obtenir la dernière version, comme si vous ouvriez la page de
  téléchargement dans un navigateur ; rien sur vous, votre jeu ou vos addons n'est envoyé.
- Les addons sont téléchargés uniquement par le programme d'addons que vous avez choisi — sauf ElvUI, que WoW Tools
  prend sur son site officiel.
- Il ne touche jamais au jeu lui-même, ne lit pas la mémoire du jeu et n'automatise rien en jeu.
- Il n'écrit dans le dossier du jeu que lorsque vous restaurez, copiez les réglages d'un personnage, faites le ménage,
  ou quand un programme d'addons met à jour les addons — et tout ce qui touche aux réglages ou aux addons crée d'abord
  une sauvegarde.

## Mettre à jour WoW Tools

WoW Tools consulte les [releases](https://github.com/Gronck2/wow-tools/releases) de ce dépôt une fois par jour (et
quand vous cliquez sur **Vérifier** dans **Paramètres → Général**). Quand une version plus récente existe :

- une notification s'affiche une fois ;
- le panneau affiche un bouton avec le numéro de la nouvelle version, à côté des icônes du dossier et des paramètres ;
- le menu propose **Mettre à jour vers x.y.z**, et les paramètres affichent **Installer**.

Un clic télécharge le nouvel exe, le vérifie avec la somme de contrôle SHA-256 que GitHub publie pour le fichier de la
release (un fichier qui ne correspond pas n'est jamais installé), le met à la place de l'actuel et redémarre l'icône de
notification. L'ancien exe est conservé sous le nom `<name>.exe.old` jusqu'au prochain démarrage, puis supprimé. Si une
sauvegarde ou une mise à jour est en cours, le redémarrage attend qu'elle se termine. Vos paramètres, vos sauvegardes
et le démarrage automatique restent tels quels.

En ligne de commande : `wow-tools self-update --check` (vérifier seulement) ou `wow-tools self-update` (vérifier et
installer ; redémarrez ensuite l'icône de notification). Pour ne plus vérifier, désactivez **Rechercher les mises à
jour** dans **Paramètres → Général** (`check_updates = false` dans `config.toml`).

## Désinstallation

1. Désactivez **Lancer avec Windows** (ou lancez `wow-tools schedule remove`, qui supprime aussi une éventuelle tâche
   planifiée).
2. Choisissez **Quitter** dans le menu de l'icône de notification.
3. Supprimez l'exe et, si vous n'avez plus besoin des paramètres ni des sauvegardes, `%APPDATA%\WoW Tools` et
   `%LOCALAPPDATA%\WoW Tools`.

## FAQ et limites

- **La restauration indique que le jeu est lancé.** Fermez cette version du jeu (le launcher Battle.net peut rester
  ouvert).
- **Les raccourcis copiés ont disparu après la connexion.** Ils sont synchronisés avec le serveur ; voir
  [plus haut](#copier-les-réglages-entre-personnages).
- **La mise à jour indique « Aucune mise à jour. », mais le programme affiche des mises à jour.** Ces addons sont
  réglés en manuel ou ignorés dans le programme, ou (WowUp) viennent de Wago — la ligne de l'historique en donne la
  raison.
- **Peut-on utiliser plusieurs programmes d'addons à la fois ?** Non, et c'est voulu : ils cassent les addons l'un de
  l'autre.
- **Où le dossier du jeu est-il cherché ?** Dans le registre, la liste des installations de Battle.net, puis les
  chemins habituels (`C:\Program Files (x86)\World of Warcraft`, `X:\Games\World of Warcraft`, …). Les versions du jeu
  que Battle.net a installées dans d'autres dossiers sont trouvées aussi. Sinon, indiquez-le dans
  **Paramètres → Général**.

## Compiler depuis les sources

```
cargo build --release
```

Rust stable, Windows. La CI (GitHub Actions) vérifie le formatage, clippy, les tests et la compilation à chaque push ;
un tag `vX.Y.Z` correspondant à la version de `Cargo.toml` compile l'exe et crée un brouillon de release. Les textes
sont dans `locales/<langue>.toml`.
