---
id: "019"
titre: "Installation au premier lancement"
etape: "7a"
statut: "implémentée"
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

Réalisation (amendement du 2026-11-22) :

- **Ordre au démarrage** : la décision se prend dans `main`, avant l'instance unique, le pipe et la passerelle.
  Le mode `stdio` ne la prend jamais ; le pont lance l'appli avec `--no-install`, si bien qu'un client LLM ne fait
  jamais apparaître la fenêtre d'installation, d'où que l'exe soit lancé.
- **Arguments internes** (analyse stricte de `gateway::launch`) : `--autostart` (démarrage par la valeur `Run` :
  aucune fenêtre au démarrage, même au premier lancement, et jamais de dialogue d'installation ; une installation
  présente ailleurs reçoit la main avec `--autostart`) et `--no-install` (l'appli depuis l'emplacement courant, sans
  rien proposer : relance après un refus ou un échec, lancement par le pont).
- **Emplacement** : l'exe est l'exe installé quand son chemin et celui de `…\Programs\MCP Proxy\mcp-proxy.exe`,
  résolus (noms courts, liens), désignent le même fichier, sans tenir compte de la casse.
- **Version de l'installation présente** : lue dans le fichier de l'exe installé, sans l'exécuter et sans
  `GetFileVersionInfo` (`unsafe`, D18) : chaque build porte une marque `\x01mcp-proxy:version=<semver>\x01`,
  cherchée dans les octets du fichier (512 Mio au plus). Marque absente, plusieurs versions différentes, fichier
  illisible : version **inconnue**, et l'exe installé n'est jamais remplacé (il pourrait être plus récent) : il reçoit
  la main, comme au cas 3.
- **Fenêtre d'installation** : page locale `install.html`, fenêtre `install` de 500 × 440, capacité `install`, une
  seule commande `answer_install`, dans une appli Tauri à part, sans instance unique ni passerelle (une instance en
  cours prendrait sinon la main). Case décochée par défaut, cochée si la valeur `Run` désigne déjà l'exe installé.
  « Installer » ou « Mettre à jour » ; « Pas maintenant » fait tourner l'appli depuis l'emplacement courant (cas 1)
  ou lance la version installée (cas 2, bouton « Lancer la version X ») ; fermer la fenêtre quitte, sauf pendant
  l'installation, où la fermeture est refusée. Un échec affiche son message et propose « Continuer sans installer »
  (l'appli depuis l'emplacement courant, relancée avec `--no-install`) ou « Quitter ».
- **Arrêt de l'instance en cours** (cas 1 comme cas 2 : une appli lancée sans installation tient le pipe) : par un
  canal de contrôle authentifié par la preuve mutuelle de la spec 011, rôle `controle`. L'appli n'accepte de ce
  rôle qu'une demande d'arrêt, journalisée, et quitte comme par « Quitter » (et, depuis la spec 024, le `ping` sans
  effet de l'auto-diagnostic) ; depuis la spec 026, cet arrêt écrit, comme tout arrêt volontaire, la marque
  `arret-volontaire` qui retient les ponts des clients LLM de relancer l'appli : ils attendent l'instance que
  l'installation lance et s'y reconnectent ; le pont prouve toujours le rôle `pont`,
  si bien que rien de ce qu'envoie un client LLM n'atteint ce canal. Le client attend, 20 s au plus, que l'appli
  ferme le canal en terminant son processus. Instance qui ne s'arrête pas : message, rien n'est modifié.
- **Étapes et retour arrière** : installation = arrêt de l'instance, dossiers créés un à un, copie sous
  `mcp-proxy.exe.new` puis renommage en `mcp-proxy.exe`, raccourci, valeur `Run` (posée si la case est cochée,
  retirée sinon), entrée « Applications installées » (spec 022), lancement de l'exe installé. Mise à jour = arrêt,
  copie sous `.new`, exe installé renommé en `mcp-proxy.exe.old` (un exe tenu par un pont se renomme, il ne se
  remplace pas), `.new` renommé en `mcp-proxy.exe`, entrée « Applications installées » à la nouvelle version,
  raccourci s'il manque, lancement, puis suppression de `.old` (laissé pour la mise à jour suivante si un pont le
  tient encore ; depuis la spec 026, cette mise à jour le renomme alors en reste `mcp-proxy.exe.old-<n>.__relocated__.exe`
  avant de mettre l'exe installé de côté) ; la valeur `Run` n'est pas touchée. Chaque étape faite est notée ; une étape qui échoue défait les
  précédentes dans l'ordre inverse (fichiers retirés, ancien exe remis, dossiers créés retirés, raccourci créé
  retiré, valeur `Run` et entrée « Applications installées » remises à l'identique) ; ce qui n'a pas pu être défait est nommé dans le message.
- **Démarrage automatique** : valeur `MCP Proxy` sous `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`,
  `"<exe installé>" --autostart`. **La valeur fait foi** (c'est elle que Windows applique) : seule la commande exacte
  de l'exe installé compte comme active ; `UserSettings::autostart` la reflète au démarrage, en mémoire seulement
  (le fichier scellé n'est pas réécrit par une simple lecture du registre). L'entrée « Démarrer avec Windows » du
  menu de l'icône relit la valeur, l'écrit ou la retire, puis enregistre `UserSettings::autostart` ; hors de
  l'installation, l'entrée est grisée : une valeur `Run` ne désigne jamais un emplacement provisoire.
- **Raccourci** : `%APPDATA%\Microsoft\Windows\Start Menu\Programs\MCP Proxy.lnk`, écrit par `mslnk` (format `.lnk`
  en Rust pur, sans COM) dans un fichier temporaire puis renommé. `mslnk` n'écrit aucun bloc de données
  supplémentaires : **pas d'AUMID** dans le raccourci, l'enregistrement par le registre suffit (spec 013).
- **Deux dossiers** : l'exe dans `%LOCALAPPDATA%\Programs\MCP Proxy\`, les données dans `%LOCALAPPDATA%\MCP Proxy\`
  (`settings.key` à côté de `settings.json`, spec 014) ; l'installation ne touche jamais aux données.
- **Journal** : étapes et genres d'erreur, jamais un chemin (il contient le nom de l'utilisateur).

### Organisation du code

- `mcp_proxy_deploy::install` : la décision (`InstallPlan` : installer, mettre à jour, déléguer, continuer) est
  une fonction pure des chemins et des versions, testée sous Linux.
- Les opérations (copie, raccourci, registre) passent par un trait, avec une implémentation Windows et une fausse
  pour les tests. Aucun `unsafe` (D18) : registre par `winreg`, raccourci par un crate sûr, plus de `IShellLinkW` ni
  de `IPropertyStore`.

Réalisation : `mcp_proxy_deploy::install` (`Situation::inspect` puis `Situation::plan`, `InstallPaths`,
`install`, `update`, `delegate`, `run_here`, `InstallError`, `InstallDialog` et `parse_answer`, `run_command`,
`set_autostart`), `mcp_proxy_deploy::version` (marque de version, comparaison semver), `mcp_proxy_deploy::ops`
(trait `SystemOps`, corps par défaut `std::fs` pour les fichiers) et `mcp_proxy_deploy::fake` (`FakeSystem`, en
mémoire, chaque opération notée et chacune pouvant échouer). Canal de contrôle : `gateway::bridge` (`Role::Control`,
`request_stop`, `serve_control`) et `Gateway::exit_requested`. `app/` : `install.rs` (décision, fenêtre, commande),
`system_ops.rs` (`WindowsOps`). `ui/` : `install.html`, `src/install/`.

## Tests

- **EX-019-01** — `InstallPlan` pour chaque situation : pas d'installation, version plus récente, identique, plus ancienne, déjà à
  l'emplacement définitif, lancement `--autostart`.
- **EX-019-02** — Avec la fausse implémentation : ordre des opérations ; échec à chaque étape → retour arrière.
- **EX-019-03** — Comparaison semver, préversions comprises.
- **EX-019-09** — Arguments de lancement `--autostart` et `--no-install` : reconnus exactement, refusés en toute
  autre casse ou avec un second argument.
- **EX-019-10** — Version de l'exe installé lue dans son fichier par la marque de version, sans l'exécuter ; marque
  absente, coupée, double et contradictoire, fichier vide, absent ou dossier : version inconnue ; la marque de ce
  build survit à l'édition de liens.
- **EX-019-11** — Canal de contrôle : un client prouvé en rôle `controle` obtient l'arrêt (journalisé) et attend la
  fermeture du canal, 20 s au plus ; un pont (rôle `pont`), un client d'un autre utilisateur, une demande autre que
  l'arrêt ou une réponse autre que l'accusé n'arrêtent rien.
- **EX-019-12** — Démarrage automatique : valeur `Run` (chemin entre guillemets, `--autostart`), seule la commande
  exacte compte comme active ; l'entrée du menu ne change rien hors de l'installation ; `UserSettings::autostart`
  reflète la valeur sans écrire les réglages.
- **EX-019-13** — Fenêtre d'installation (navigateur, backend simulé) : installation avec la case, « Pas
  maintenant », proposition de mise à jour, échec et ses deux issues, données lues, réponses envoyées par sa seule
  commande ; rien ne déborde de la fenêtre, en clair et en sombre.
- **EX-019-14** — Réponses de la fenêtre : forme stricte, réponse hors de propos ignorée, une seule action à la fois,
  fermeture refusée pendant l'installation ; données de la page sans aucun chemin.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-019-04** (manuel) — Lancer l'exe depuis le dossier Téléchargements : la boîte d'installation apparaît ; après validation, l'appli
  tourne depuis `%LOCALAPPDATA%\Programs\MCP Proxy\` et le raccourci existe dans le menu Démarrer.
- **EX-019-05** (manuel) — Case cochée : l'appli démarre à l'ouverture de session, sans fenêtre.
- **EX-019-06** (manuel) — Relancer l'exe téléchargé : l'exe installé est utilisé, pas de seconde installation.
- **EX-019-07** (manuel) — Aucune demande d'élévation à aucun moment.
- **EX-019-08** (manuel) — Noter le comportement de SmartScreen sur un exe non signé (voir `028-authenticode.md`).
- **EX-019-15** (manuel) — Mise à jour avec l'appli installée en cours et un client LLM connecté : la fenêtre la
  propose ; « Mettre à jour » ferme l'appli en cours, remplace l'exe et relance la nouvelle version ; « Lancer la
  version X » lance la version installée sans rien remplacer.
- **EX-019-16** (manuel) — Échec : dossier d'installation verrouillé ou en lecture seule, ou appli en cours qui ne
  s'arrête pas : message clair, aucun reste (dossier, raccourci, valeur `Run`, copie), et « Continuer sans
  installer » fait tourner l'appli depuis l'emplacement courant.
- **EX-019-17** (manuel) — Entrée « Démarrer avec Windows » du menu de l'icône : la décocher retire la valeur `Run`,
  la cocher la remet ; grisée quand l'appli tourne hors de son installation.
- **EX-019-18** (manuel) — Un client LLM configuré sur l'exe du dossier Téléchargements (`stdio`) : aucune fenêtre
  d'installation, l'appli démarre depuis cet emplacement.

## Hors périmètre

Mises à jour (7b). Désinstallation (7d).

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur). Raccourci par un crate sûr (par exemple
  `mslnk`), sans AUMID s'il ne sait pas le poser, puisque le registre suffit ; registre par `winreg` ; dialogue
  par une API sûre ; plus de `IShellLinkW`. Aucun critère touché.
- **2026-11-22** — Réalisation (I019.1 à I019.14). Ajouts : section « Réalisation » du déroulé et de l'organisation
  du code ; critères `EX-019-09` à `EX-019-14` (tests) et `EX-019-15` à `EX-019-18` (recette). Écarts et précisions :
  arguments internes `--autostart` et `--no-install` ; le pont lance l'appli avec `--no-install` ; la version de
  l'installation présente est lue par une marque dans le fichier, et une version illisible n'est jamais remplacée ;
  l'instance en cours est arrêtée aussi avant une première installation (une appli lancée sans installation tient le
  pipe), par un rôle `controle` de la preuve mutuelle de la spec 011 ; la mise à jour met l'ancien exe de côté par
  renommage ; la valeur `Run` fait foi et `UserSettings::autostart` la reflète ; le raccourci n'a pas d'AUMID
  (`mslnk` n'écrit pas de bloc de propriétés). Raison : points laissés ouverts par le texte, tranchés par l'option la
  plus sûre, et contraintes de D18.
- **2026-11-23** — Spec 022 : l'installation et la mise à jour écrivent l'entrée « Applications installées »
  (`HKCU\…\Uninstall\MCP Proxy`) avec la version lue dans l'exe mis en place, comme une étape de la transaction
  (défaite par un retour arrière : entrée retirée, ou remise à l'identique si elle existait). Les tests d'ordre et de
  retour arrière (`EX-019-02`) comptent cette étape. Aucun critère touché.
- **2026-11-23** — Spec 024 : le canal de contrôle accepte aussi `ping`, la demande sans effet de l'auto-diagnostic
  (l'appli répond sa version et ferme le canal : ni arrêt, ni journal). Aucun critère de la 019 touché ; `EX-024-10`
  le prouve.
- **2026-11-23** — Spec 026 : l'arrêt demandé par le canal de contrôle écrit, comme « Quitter », la marque
  `arret-volontaire` du dossier de données ; les ponts des clients LLM ne relancent donc pas l'ancienne version
  pendant l'installation ou la mise à jour et se reconnectent à celle que l'installation lance, sans que le client
  soit redémarré. La limite connue d'I019.7 ne vaut plus que pour un pont lancé pendant ces secondes par un client
  qui démarre. Aucun critère touché ; la recette `EX-019-15` ne demande plus de relancer le serveur MCP du client.
- **2026-11-23** — Spec 026 : à la mise à jour, un `mcp-proxy.exe.old` encore exécuté par un pont est renommé en reste
  (`mcp-proxy.exe.old-<n>.__relocated__.exe`) au lieu d'empêcher la mise de côté de l'exe installé. Prouvé par
  `EX-026-20` ; aucun critère de la 019 touché.
