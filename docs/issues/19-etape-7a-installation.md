# [Étape 7a] Installation au premier lancement

> Référence commune : `00-cadrage.md`. Dépend de : étape 5a.

## Objectif

Un seul exe, sans droits admin : au premier lancement, il s'installe lui-même puis redémarre depuis son
emplacement définitif.

## À faire

### Déroulé

Au lancement, si l'exe ne s'exécute pas depuis `%LOCALAPPDATA%\Programs\MCP Proxy\` :

1. **Aucune installation présente** : petite boîte de dialogue native « Installer MCP Proxy » avec la case
   « Démarrer MCP Proxy avec Windows » (l'option de démarrage automatique). Puis :
   - copie de l'exe vers `%LOCALAPPDATA%\Programs\MCP Proxy\mcp-proxy.exe` ;
   - raccourci `MCP Proxy.lnk` dans le menu Démarrer de l'utilisateur, portant l'AUMID
     (`System.AppUserModel.ID`) ;
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
  pour les tests. Raccourci : `IShellLinkW` + `IPropertyStore`.

## Tests

- `InstallPlan` pour chaque situation : pas d'installation, version plus récente, identique, plus ancienne, déjà à
  l'emplacement définitif, lancement `--autostart`.
- Avec la fausse implémentation : ordre des opérations ; échec à chaque étape → retour arrière.
- Comparaison semver, préversions comprises.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- Lancer l'exe depuis le dossier Téléchargements : la boîte d'installation apparaît ; après validation, l'appli
  tourne depuis `%LOCALAPPDATA%\Programs\MCP Proxy\` et le raccourci existe dans le menu Démarrer.
- Case cochée : l'appli démarre à l'ouverture de session, sans fenêtre.
- Relancer l'exe téléchargé : l'exe installé est utilisé, pas de seconde installation.
- Aucune demande d'élévation à aucun moment.
- Noter le comportement de SmartScreen sur un exe non signé (voir `28-backlog-authenticode.md`).

## Hors périmètre

Mises à jour (7b). Désinstallation (7d).
