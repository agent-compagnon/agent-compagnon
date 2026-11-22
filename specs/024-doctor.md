---
id: "024"
titre: "Auto-diagnostic (`doctor`)"
etape: "7f"
statut: "prévue"
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
- Entrée « Auto-diagnostic » dans le menu de l'en-tête : même rapport dans un dialogue, avec « Copier ».
- Le rapport est joint à l'export de diagnostic (3e).

### Contrôles

Chaque contrôle rend `ok`, `avertissement` ou `échec`, un message, et **quoi faire** pour corriger.

| Contrôle | Détail |
|---|---|
| Config | Chargée et valide |
| WebView2 | Runtime présent, version suffisante pour le mode InPrivate |
| Installation | Exe à l'emplacement attendu, dossier inscriptible, raccourci et AUMID (registre) présents, exclusion WER posée |
| Notifications | Toasts activés pour l'appli |
| Appli | Instance en cours, pipe joignable |
| Réglages | Sceau valide (5d) |
| Artifactory | API Storage joignable, dernière version disponible |
| Clé de mise à jour | Ce n'est pas la clé de développement |
| Clients LLM | Détectés, configurés ou non (7e) |
| MCP | Pour chaque instance exposée : résolution de nom et TLS de son origine. **Aucun identifiant n'est envoyé** ; toute réponse HTTP compte comme joignable |
| Horloge | Écart manifeste avec l'heure d'un serveur (les échéances des jetons en dépendent) |

- Aucun secret dans le rapport. Il passe par le filtre comme le reste.

### Organisation du code

- `mcp_proxy_deploy::doctor` : trait `Check`, sondes de plateforme derrière des traits, rendu texte et structuré.

## Tests

- **EX-024-01** — Chaque contrôle dans ses trois issues, avec de fausses sondes.
- **EX-024-02** — Code de sortie selon le pire résultat.
- **EX-024-03** — Le contrôle MCP n'envoie aucun en-tête d'authentification (faux serveur qui enregistre les requêtes).
- **EX-024-04** — Rapport sans secret (valeur sentinelle).
- **EX-024-08** — Sortie du rapport (faux système de fichiers, fausse sortie standard) : le rapport est écrit dans un
  nouveau fichier du dossier des logs, dont le chemin figure à la fin du rapport, et le même texte est écrit sur la
  sortie standard ; un fichier impossible à écrire donne un échec signalé sur la sortie standard et le code 2.

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
