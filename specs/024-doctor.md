---
id: "024"
titre: "Auto-diagnostic (`doctor`)"
etape: "7f"
statut: "en cours"
depend_de: ["009", "011", "019", "020"]
suivi: "github#25"
---

# 024 — Auto-diagnostic (`doctor`)

> Option retenue au QCM (D14).

## Objectif

Qu'un utilisateur ou le support sache en quelques secondes pourquoi « ça ne marche pas », sans lire de logs.

## À faire

### Trois accès

- `mcp-proxy.exe doctor` : rapport texte et code de sortie (0 tout va bien, 1 avertissements, 2 échecs). L'exe est
  en sous-système fenêtré et ne se rattache pas à la console parente (`AttachConsole` n'existe qu'en `unsafe`,
  D18). Le rapport est écrit :
  - sur la sortie standard, visible quand elle est redirigée (`mcp-proxy.exe doctor > rapport.txt`,
    `mcp-proxy.exe doctor | more`) ;
  - **toujours** dans un fichier du dossier des logs (`%LOCALAPPDATA%\MCP Proxy\logs\doctor-<horodatage>.txt`),
    dont le chemin termine le rapport et est indiqué dans une boîte de message (API sûre) quand la commande est
    lancée sans redirection.
  `doctor` est lancé avant l'instance unique, comme `--uninstall` : il ne passe pas la main à l'instance en cours et
  ne montre pas sa fenêtre. « Sans redirection » se reconnaît sans `unsafe` : la poignée de sortie standard d'un exe
  fenêtré lancé sans redirection est nulle (`std::io::stdout().as_raw_handle()`, une comparaison). Le fichier
  s'appelle `doctor-AAAAMMJJ-HHMMSS.txt` (UTC), suivi de `_2` à `_9` dans la même seconde : jamais un rapport
  précédent remplacé. Son chemin est écrit depuis `%LOCALAPPDATA%`, sans le nom de l'utilisateur. Un fichier
  impossible à écrire est signalé à la fin du rapport, sur la sortie standard, et donne le code 2.
- Entrée « Auto-diagnostic » dans le menu de l'en-tête : même rapport dans un dialogue, avec « Copier ». Le rapport
  est aussi écrit dans un nouveau fichier du dossier des logs.
- Le rapport est joint à l'export de diagnostic (3e) : il est **refait au moment de l'export** (un fichier d'un
  autre jour induirait le support en erreur), sans écrire de fichier de rapport.

### Ce que le diagnostic s'interdit

- **Il ne modifie rien.** Aucun contrôle ne répare, ne crée ni ne réécrit : la clé de scellement est ouverte, jamais
  créée ; le fichier des réglages est lu, jamais réécrit ; le registre est lu (`winreg`, accès en lecture) ; l'état
  des notifications est lu sans enregistrer l'AUMID ; les fichiers des clients LLM sont lus. La seule écriture est le
  fichier du rapport (et le dossier des logs s'il manque). Rien n'est journalisé.
- **Le pipe est sondé par une demande sans effet** : le canal de contrôle de la spec 019 gagne `ping`, après la
  preuve mutuelle (rôle `controle`). L'appli répond sa version et ferme le canal : ni arrêt, ni fenêtre, ni journal.
  Une version antérieure, qui ne connaît que `stop`, ferme le canal après la preuve : elle compte comme joignable.
- **Le réseau** passe par un client à part, anonyme, jamais celui des sessions ni celui de l'updater : ni magasin de
  cookies, ni en-tête d'authentification, ni `Referer`, aucune redirection suivie (une redirection est une réponse
  comme une autre), adresse avec identifiants refusée, HTTPS (HTTP vers la boucle locale seulement). Seules les
  **origines** des instances exposées (profil choisi, environnement exposé, MCP non coupé) et l'API Storage
  d'Artifactory sont contactées, une requête par origine. Délais bornés (connexion 5 s, requête 8 s, résolution
  5 s) ; les contrôles tournent ensemble, chacun borné à 12 s : la durée totale l'est aussi.
- **Aucun secret** : chaque texte passe par le filtre de sortie avant de sortir (dialogue, fichier, sortie standard,
  archive). En ligne de commande, le processus ne détient aucun secret : le filtre a un store vide, ses motifs
  génériques (JWT, clés privées, en-têtes…) s'appliquent. Filtre indisponible : aucun rapport, une ligne fixe, code
  2. Ni URL complète (l'hôte seulement), ni contenu de réponse, ni contenu des fichiers des clients LLM.

### Contrôles

Chaque contrôle rend `ok`, `avertissement` ou `échec`, un message, et **quoi faire** pour corriger.

| Contrôle | Détail |
|---|---|
| Config | Chargée et valide |
| WebView2 | Runtime présent, version suffisante pour le mode InPrivate |
| Installation | Exe à l'emplacement attendu, dossier inscriptible, raccourci et AUMID (registre) présents, exclusion WER posée, entrée « Applications installées » à la version de l'exe (spec 022) |
| Notifications | Toasts activés pour l'appli |
| Appli | Instance en cours, pipe joignable |
| Réglages | Sceau valide (5d) |
| Artifactory | API Storage joignable, dernière version disponible |
| Clé de mise à jour | Ce n'est pas la clé de développement |
| Clients LLM | Détectés, configurés ou non (7e) |
| MCP | Pour chaque instance exposée : résolution de nom et TLS de son origine. **Aucun identifiant n'est envoyé** ; toute réponse HTTP compte comme joignable |
| Horloge | Écart manifeste avec l'heure d'un serveur (les échéances des jetons en dépendent) |

- Aucun secret dans le rapport. Il passe par le filtre comme le reste.

Ce que chaque contrôle signale, en plus du tableau :

- **Config** : avertissement si des adresses désignent des noms réservés aux exemples (`example.internal`, `.test`…) :
  la configuration embarquée est celle d'exemple.
- **WebView2** : runtime trop ancien pour InPrivate, échec avec le profil `in_memory` (toute connexion par fenêtre est
  refusée), avertissement avec `ephemeral_disk`.
- **Installation** : version de l'exe installé illisible (échec : jamais mise à jour) ; `mcp-proxy.exe.old` ou `.new`
  resté ; valeur `Run` qui ne désigne pas l'exe installé ; entrée « Applications installées » absente, d'un autre
  exe ou d'une autre version (avertissement : réparée au démarrage suivant de l'exe installé) ; raccourci, AUMID ou
  exclusion WER absents ; dossier ou exe en lecture seule. « Dossier inscriptible » se lit à l'attribut lecture
  seule, sans écrire.
- **Notifications** : désactivées pour l'appli, pour toutes les applications, par une stratégie, ou « Ne pas
  déranger » : avertissement (les confirmations s'ouvrent en fenêtre) ; état illisible : échec.
- **Appli** : en ligne de commande, instance absente (avertissement : un client LLM la lance), canal tenu par un autre
  programme ou muet (échec), instance plus ancienne que l'exe installé (avertissement : une mise à jour attend son
  redémarrage). **Vu de l'appli**, l'instance est celle-ci : le contrôle sonde son propre pipe comme un client le
  ferait, et un pipe qui ne répond pas est un échec.
- **Réglages** : sceau refusé (échec) ; premier lancement, aucun profil, aucun environnement exposé, passerelle
  suspendue (avertissements). Vu de l'appli, un fichier refusé dit que les réglages en cours s'appliquent jusqu'à
  l'arrêt.
- **Artifactory** : 404 ou aucune version publiée (avertissement), autre statut, liste illisible ou serveur
  injoignable (échec) ; version plus récente disponible alors que l'exe ne se met pas à jour seul (avertissement).
- **Clé de mise à jour** : aucune clé embarquée, ce qui désactive les mises à jour (avertissement, le cas des
  constructions hors pipeline) ; clé de développement dans un emplacement (échec).
- **Clients LLM** : état de chaque emplacement de la spec 023, les deux de Claude Desktop compris (quand les deux
  existent, celui de `%APPDATA%` est dit « sans doute pas lu ») ; fichier illisible (échec), entrée d'un autre exe,
  aucun client configuré ou détecté (avertissement).
- **MCP** : réponse 5xx (avertissement : serveur en erreur ou proxy qui ne le joint pas) ; nom introuvable, TLS
  refusé, connexion impossible, délai (échec). Aucune instance exposée : rien n'est contacté.
- **Horloge** : écart entre l'horloge du poste et l'en-tête `Date` des réponses des autres contrôles (aucune requête
  de plus) ; le plus petit écart compte. Plus de 60 s : avertissement ; plus de 5 min : échec.

### Organisation du code

- `mcp_proxy_deploy::doctor` : trait `Check`, sondes de plateforme derrière des traits, rendu texte et structuré.
- `mcp_proxy_gateway::doctor` : faits de la configuration, des réglages (lus sans effet) et de l'exposition, filtre,
  fichier du rapport ; `Gateway::diagnose` vu de l'appli, `run_command` pour `mcp-proxy.exe doctor`.
- `app/src/doctor.rs` : sondes Windows (toasts par WinRT, pipe par `ping`, registre par `WindowsOps`), commande
  `run_doctor` de la fenêtre `main`.

## Tests

- **EX-024-01** — Chaque contrôle dans ses trois issues, avec de fausses sondes.
- **EX-024-02** — Code de sortie selon le pire résultat.
- **EX-024-03** — Le contrôle MCP n'envoie aucun en-tête d'authentification (faux serveur qui enregistre les requêtes).
- **EX-024-04** — Rapport sans secret (valeur sentinelle).
- **EX-024-08** — Sortie du rapport (faux système de fichiers, fausse sortie standard) : le rapport est écrit dans un
  nouveau fichier du dossier des logs, dont le chemin figure à la fin du rapport, et le même texte est écrit sur la
  sortie standard ; un fichier impossible à écrire donne un échec signalé sur la sortie standard et le code 2.
- **EX-024-10** — Le `ping` du canal de contrôle ne fait que répondre la version : ni arrêt, ni journal, ni écriture
  des réglages ; refusé à un autre utilisateur ; une version qui ne le connaît pas se reconnaît à la fermeture du
  canal.
- **EX-024-11** — Le diagnostic ne modifie rien : aucune écriture au registre ni dans les fichiers des clients LLM
  (fausses implémentations qui comptent les écritures), réglages et clé de scellement inchangés octet pour octet,
  aucune clé créée au premier lancement ; seul le rapport est écrit ; seules les origines des instances exposées et
  Artifactory sont contactées.
- **EX-024-12** — Le dialogue « Auto-diagnostic » montre le contrôle en cours, puis chaque contrôle avec son issue et
  la marche à suivre ; « Copier » copie le texte du rapport (contrat `ui/src/api/doctor.example.json`).
- **EX-024-13** — L'export de diagnostic joint un rapport refait au moment de l'export, filtré, sans écrire de
  fichier de rapport.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- ~~**EX-024-05** (manuel) — `mcp-proxy.exe doctor` dans un terminal affiche le rapport.~~ (retiré le 2026-11-22 :
  D18, plus de rattachement à la console ; remplacé par `EX-024-09`)
- **EX-024-06** (manuel) — Désactiver les notifications de l'appli dans Windows : le contrôle passe en avertissement avec la marche à
  suivre.
- **EX-024-07** (manuel) — Le dialogue de l'interface affiche le même rapport.
- **EX-024-09** (manuel) — Dans un terminal, `mcp-proxy.exe doctor > rapport.txt` puis `echo %ERRORLEVEL%` : le
  fichier contient le rapport et le code de sortie correspond ; `mcp-proxy.exe doctor` sans redirection écrit le
  fichier du dossier des logs et indique son chemin.

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur) : plus de rattachement à la console. Le rapport
  est écrit sur la sortie standard (visible quand elle est redirigée) et toujours dans un fichier du dossier des
  logs dont le chemin est indiqué. Le contrôle « Installation » vérifie aussi l'exclusion WER. Critère retiré :
  `EX-024-05` ; ajoutés : `EX-024-08` et `EX-024-09`.
- **2026-11-23** — Spec 022 : le contrôle « Installation » vérifie aussi l'entrée « Applications installées »
  (`HKCU\…\Uninstall\MCP Proxy`) : présente, `UninstallString` qui désigne l'exe installé, `DisplayVersion` égale à la
  marque de version de l'exe. De quoi s'en servir : `SystemOps::uninstall_entry` et `uninstall::entry_for` (valeurs
  attendues), `uninstall::launch` (l'exe lancé est-il l'exe installé). Une entrée absente se répare d'elle-même au
  démarrage de l'exe installé : avertissement, pas échec. Aucun critère touché.
- **2026-11-23** — Implémentation. Écarts et précisions : le pipe est sondé par une demande sans effet ajoutée au
  canal de contrôle (`ping`, après la preuve mutuelle), « dossier inscriptible » se lit à l'attribut lecture seule
  (écrire un fichier d'essai serait une modification), le contrôle « Appli » vu de l'appli sonde son propre pipe,
  « Réglages » signale aussi l'absence de profil, d'environnement exposé et la suspension, « Config » la
  configuration d'exemple, « Horloge » se mesure aux en-têtes `Date` des autres contrôles. Le rapport du dialogue
  est aussi écrit dans un fichier ; celui de l'export est refait à l'export. Nom du fichier précisé
  (`doctor-AAAAMMJJ-HHMMSS.txt`, `_2` à `_9`). Rien n'est journalisé. Critères ajoutés : `EX-024-10` à `EX-024-13`.
