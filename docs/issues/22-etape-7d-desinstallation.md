# [Étape 7d] Désinstallation propre

> Référence commune : `00-cadrage.md`. Dépend de : étape 7a. Option retenue au QCM (D14).

## Objectif

Que l'appli se retire entièrement, depuis « Applications installées » de Windows, sans droits admin.

## À faire

### Entrée « Applications installées »

- À l'installation (7a) et à chaque mise à jour : `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy`
  avec `DisplayName`, `DisplayVersion`, `Publisher`, `InstallLocation`, `DisplayIcon`, `UninstallString`
  (`"…\mcp-proxy.exe" --uninstall`), `NoModify`, `NoRepair`, `EstimatedSize`.

### `mcp-proxy.exe --uninstall`

1. Boîte de confirmation native.
2. Arrêt de l'instance en cours, s'il y en a une.
3. Suppression : raccourci du menu Démarrer, démarrage automatique, clé AUMID, entrée de désinstallation, dossier
   de données `%LOCALAPPDATA%\MCP Proxy\` (logs, cache, profil WebView, réglages, téléchargements).
4. Entrées ajoutées aux clients LLM (7e) : proposer de les retirer, case cochée par défaut.
5. Suppression de l'exe et du dossier d'installation (`self_replace::self_delete_outside_path`).

- Aucun secret à effacer : ils n'ont jamais été sur disque.
- Chaque étape tolère que l'élément soit déjà absent. Un échec partiel est signalé avec ce qui reste.

### Organisation du code

- `mcp_proxy_deploy::uninstall` : plan de désinstallation calculé par une fonction pure ; opérations derrière le
  même trait qu'en 7a.

## Tests

- Plan complet ; éléments déjà absents ; échec à une étape → les suivantes sont tentées, le rapport liste les
  restes.
- L'entrée de registre porte la bonne version après une mise à jour.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- L'appli apparaît dans « Applications installées » avec la bonne version.
- Désinstaller : plus de raccourci, plus de démarrage automatique, plus de dossier d'installation ni de données,
  plus d'entrée dans la liste.
- Aucune demande d'élévation.
