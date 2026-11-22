---
id: "022"
titre: "Désinstallation propre"
etape: "7d"
statut: "prévue"
depend_de: ["019"]
suivi: "github#23"
---

# 022 — Désinstallation propre

> Option retenue au QCM (D14).

## Objectif

Que l'appli se retire entièrement, depuis « Applications installées » de Windows, sans droits admin.

## À faire

### Entrée « Applications installées »

- À l'installation (7a) et à chaque mise à jour : `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy`
  avec `DisplayName`, `DisplayVersion`, `Publisher`, `InstallLocation`, `DisplayIcon`, `UninstallString`
  (`"…\mcp-proxy.exe" --uninstall`), `NoModify`, `NoRepair`, `EstimatedSize`.

### `mcp-proxy.exe --uninstall`

1. Boîte de confirmation, par une API sûre (dialogue de Tauri ou équivalent).
2. Arrêt de l'instance en cours, s'il y en a une.
3. Suppression : raccourci du menu Démarrer, démarrage automatique, clé AUMID, valeur d'exclusion WER (spec 011),
   entrée de désinstallation, dossier
   de données `%LOCALAPPDATA%\MCP Proxy\` (logs, cache, profil WebView, réglages, téléchargements).
4. Entrées ajoutées aux clients LLM (7e) : proposer de les retirer, case cochée par défaut.
5. Suppression de l'exe et du dossier d'installation (`self_replace::self_delete_outside_path`).

- Aucun secret à effacer : ils n'ont jamais été sur disque.
- Chaque étape tolère que l'élément soit déjà absent. Un échec partiel est signalé avec ce qui reste.

### Organisation du code

- `mcp_proxy_deploy::uninstall` : plan de désinstallation calculé par une fonction pure ; opérations derrière le
  même trait qu'en 7a. Aucun `unsafe` (D18) : registre par `winreg` (entrée de désinstallation, clé `Run`, clé AUMID,
  exclusion WER), raccourci supprimé comme un fichier, plus de `IShellLinkW`.

## Tests

- **EX-022-01** — Plan complet ; éléments déjà absents ; échec à une étape → les suivantes sont tentées, le rapport liste les
  restes.
- **EX-022-02** — L'entrée de registre porte la bonne version après une mise à jour.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-022-03** (manuel) — L'appli apparaît dans « Applications installées » avec la bonne version.
- **EX-022-04** (manuel) — Désinstaller : plus de raccourci, plus de démarrage automatique, plus de dossier d'installation ni de données,
  plus d'entrée dans la liste.
- **EX-022-05** (manuel) — Aucune demande d'élévation.

## Amendements

- **2026-11-22** — Aucun `unsafe` (D18, demande de l'utilisateur) : registre par `winreg`, dialogue par une API
  sûre, raccourci supprimé comme un simple fichier ; la valeur d'exclusion WER posée par l'appli (spec 011) est
  retirée elle aussi. Aucun critère touché.
