# [Étape 7e] Configuration automatique des clients LLM

> Référence commune : `00-cadrage.md`. Dépend de : étapes 6a et 7a. Option retenue au QCM (D14).

## Objectif

Brancher MCP Proxy dans Claude Desktop ou VS Code en un clic, sans faire éditer de JSON à l'utilisateur, et sans
jamais abîmer sa configuration.

## À faire

### Interface

- Entrée « Clients LLM… » dans le menu de l'en-tête. Elle ouvre un dialogue listant les clients, avec pour chacun
  son état (non installé, non configuré, configuré, configuré avec un autre chemin) et les actions « Installer »
  et « Retirer ».
- Bouton « Copier la configuration » : extrait JSON générique, pour tout autre client.
- Après une installation : message indiquant de redémarrer le client.

### Adaptateurs

Un adaptateur par client, isolé, pour qu'un changement de format chez un éditeur se corrige sans toucher au reste.
**Revérifier les chemins et formats dans la documentation de chaque client au moment de coder.**

| Client | Fichier | Entrée |
|---|---|---|
| Claude Desktop | `%APPDATA%\Claude\claude_desktop_config.json` | `mcpServers."mcp-proxy" = { "command": "<exe installé>", "args": ["stdio"] }` |
| VS Code | `%APPDATA%\Code\User\mcp.json` | `servers."mcp-proxy" = { "type": "stdio", "command": "<exe installé>", "args": ["stdio"] }` |

### Écriture sûre

- **Aperçu avant écriture** : le dialogue montre la différence, l'utilisateur valide.
- Sauvegarde `<fichier>.bak-<horodatage>` avant toute modification.
- Lecture tolérante au JSON avec commentaires et virgules finales. Le reste du fichier est conservé ; seule
  l'entrée `mcp-proxy` est touchée, jamais celle d'un autre serveur.
- Si le fichier ne peut pas être lu de façon sûre : **ne rien écrire**, afficher l'extrait à coller à la main.
- Écriture atomique (fichier temporaire puis renommage). Opération idempotente.
- Fichier absent : le créer seulement si le dossier du client existe.

### Organisation du code

- `mcp_proxy_deploy::clients` : détection, calcul de la modification, écriture. Chemins injectés, donc testable
  sous Linux.

## Tests

Avec des fichiers d'exemple :

- fichier absent, vide, avec d'autres serveurs, avec une entrée `mcp-proxy` ancienne, avec commentaires et virgules
  finales ;
- les autres entrées et les autres clés sont intactes après installation et après retrait ;
- fichier illisible : rien n'est écrit, l'extrait manuel est proposé ;
- sauvegarde créée ; deux installations de suite donnent le même fichier ;
- détection de chaque état.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- « Installer » pour Claude Desktop, redémarrer Claude Desktop : les outils de MCP Proxy apparaissent.
- Idem pour VS Code.
- « Retirer » enlève l'entrée et laisse les autres serveurs en place.
