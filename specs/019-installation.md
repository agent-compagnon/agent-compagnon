---
id: "019"
titre: "Installation au premier lancement"
etape: "7a"
statut: "prévue"
depend_de: ["011"]
suivi: "github#20"
---

# 019 — Installation au premier lancement

## Objectif

Un seul exe, sans droits admin : au premier lancement, il s'installe lui-même puis redémarre depuis son
emplacement définitif.

## À faire

### Déroulé

Au lancement, si l'exe ne s'exécute pas depuis `%LOCALAPPDATA%\Programs\MCP Proxy\` :

1. **Aucune installation présente** : petite boîte de dialogue « Installer MCP Proxy », par une API sûre (dialogue
   de Tauri ou petite fenêtre locale de l'appli), avec la case « Démarrer MCP Proxy avec Windows » (l'option de
   démarrage automatique). Puis :
   - copie de l'exe vers `%LOCALAPPDATA%\Programs\MCP Proxy\mcp-proxy.exe` ;
   - raccourci `MCP Proxy.lnk` dans le menu Démarrer de l'utilisateur, écrit par un crate sûr (choisi à
     l'implémentation, par exemple `mslnk`). S'il sait poser l'AUMID (`System.AppUserModel.ID`), le poser ; sinon,
     raccourci sans AUMID : l'AUMID enregistré par le registre suffit aux toasts (spec 013) ;
   - démarrage automatique si coché (clé `Run` de `HKCU`, lancement avec `--autostart`) ;
   - **relance depuis le nouvel emplacement**, puis arrêt de l'exe d'origine.
2. **Installation présente, exe lancé plus récent** (semver) : proposer de mettre à jour l'installation. Si
   l'instance installée tourne, lui demander de s'arrêter, remplacer, relancer.
3. **Installation présente, exe lancé identique ou plus ancien** : lancer l'exe installé et s'arrêter.

- Tout se fait dans `HKCU` et `%LOCALAPPDATA%` : aucune élévation.
- Lancé avec `--autostart`, l'appli démarre sans fenêtre.
- Le démarrage automatique reste modifiable ensuite : entrée à cocher « Démarrer avec Windows » dans le menu de
  l'icône, reflétée dans `UserSettings::autostart`.
- Échec en cours d'installation (copie impossible, dossier verrouillé) : message clair, rien de partiel laissé
  derrière, et l'appli peut tout de même tourner depuis l'emplacement courant.

### Organisation du code

- `mcp_proxy_deploy::install` : la décision (`InstallPlan` : installer, mettre à jour, déléguer, continuer) est
  une fonction pure des chemins et des versions, testée sous Linux.
- Les opérations (copie, raccourci, registre) passent par un trait, avec une implémentation Windows et une fausse
  pour les tests. Aucun `unsafe` (D18) : registre par `winreg`, raccourci par un crate sûr, plus de `IShellLinkW` ni
  de `IPropertyStore`.

## Tests

- **EX-019-01** — `InstallPlan` pour chaque situation : pas d'installation, version plus récente, identique, plus ancienne, déjà à
  l'emplacement définitif, lancement `--autostart`.
- **EX-019-02** — Avec la fausse implémentation : ordre des opérations ; échec à chaque étape → retour arrière.
- **EX-019-03** — Comparaison semver, préversions comprises.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-019-04** (manuel) — Lancer l'exe depuis le dossier Téléchargements : la boîte d'installation apparaît ; après validation, l'appli
  tourne depuis `%LOCALAPPDATA%\Programs\MCP Proxy\` et le raccourci existe dans le menu Démarrer.
- **EX-019-05** (manuel) — Case cochée : l'appli démarre à l'ouverture de session, sans fenêtre.
- **EX-019-06** (manuel) — Relancer l'exe téléchargé : l'exe installé est utilisé, pas de seconde installation.
- **EX-019-07** (manuel) — Aucune demande d'élévation à aucun moment.
- **EX-019-08** (manuel) — Noter le comportement de SmartScreen sur un exe non signé (voir `028-authenticode.md`).

## Hors périmètre

Mises à jour (7b). Désinstallation (7d).

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur). Raccourci par un crate sûr (par exemple
  `mslnk`), sans AUMID s'il ne sait pas le poser, puisque le registre suffit ; registre par `winreg` ; dialogue
  par une API sûre ; plus de `IShellLinkW`. Aucun critère touché.
