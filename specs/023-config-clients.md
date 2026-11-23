---
id: "023"
titre: "Configuration automatique des clients LLM"
etape: "7e"
statut: "implémentée"
depend_de: ["016", "019"]
suivi: "github#24"
---

# 023 — Configuration automatique des clients LLM

> Option retenue au QCM (D14).

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

Réalisation (amendement du 2026-11-23) :

- **Menu de l'en-tête** : bouton « Menu » (icône) à droite du sélecteur de profil, menu déroulant de Reka UI (pas le
  composant `select`, I017.9) ; une entrée, « Clients LLM… », à laquelle la spec 024 ajoutera « Auto-diagnostic »
  (`ui/src/components/AppMenu.vue`).
- **Dialogue** (`ui/src/components/clients/`) : une ligne par **emplacement** (Claude Desktop en a deux, voir
  « Adaptateurs ») avec le client, la variante (« installation classique », « paquet MSIX (installeur officiel) »,
  « profil par défaut »), le fichier écrit depuis `%APPDATA%` ou `%LOCALAPPDATA%` (jamais le nom de
  l'utilisateur), l'état en toutes lettres, le nombre d'autres serveurs du fichier, puis « Installer » et
  « Retirer ». Un cinquième état : **fichier illisible**, avec sa raison (« JSON invalide à la ligne 12 »).
  « Configuré avec un autre chemin » nomme la commande trouvée et dit que « Installer » la remplace par l'exe
  installé, après aperçu. Un client non installé n'a aucun bouton ; « Installer » est inactif quand l'entrée est
  déjà la bonne ; « Retirer » n'est actif que s'il y a une entrée, ou un fichier illisible (l'aperçu dit alors quoi
  retirer à la main). Couleurs : orange pour « configuré avec un autre chemin » (l'entrée attend « Installer »),
  rouge pour « fichier illisible » et les erreurs, noir et blanc pour le reste.
- **Aperçu** : avant toute écriture, le fichier visé, ce qui change (« créé avec cette seule entrée », « ajoutée »,
  « remplacée », « retirée », « rien à écrire »), la différence (ligne `-` de l'entrée remplacée ou retirée : sa
  commande et les **noms** de ses autres champs, jamais leurs valeurs ; lignes `+` de l'entrée écrite), « Les N
  autres serveurs et le reste du fichier ne sont pas modifiés. » et la sauvegarde. « Installer » ou « Retirer »
  renvoie l'**empreinte** de l'aperçu : si le fichier, l'exe installé ou le changement ne sont plus ceux de
  l'aperçu, rien n'est écrit et le dialogue montre l'aperçu à jour, en le disant.
- **Après l'écriture** : nom de la sauvegarde, puis le redémarrage à faire : Claude Desktop, le quitter depuis son
  icône de la zone de notification (clic droit, « Quit »), et non en fermant seulement sa fenêtre ; VS Code, le
  redémarrer ou recharger sa fenêtre (« Developer: Reload Window »).
- **Fichier illisible** : l'aperçu dit pourquoi, n'offre pas d'écriture, et donne la configuration à coller avec
  « Copier l'entrée » (pour « Retirer » : l'entrée à retirer à la main).
- **MCP Proxy non installé** (lancé avec `--no-install`, exe installé absent) : la commande à écrire n'existe pas.
  Option la plus sûre : rien n'est ajouté ni copié (pas de chemin provisoire dans un client, qui cesserait de
  marcher dès que l'exe téléchargé bouge) ; le dialogue le dit et « Retirer » reste possible.
- **Copier la configuration** : `{ "mcpServers": { "mcp-proxy": { "command": …, "args": ["stdio"] } } }`, forme la
  plus courante, par le presse-papiers de la WebView (`navigator.clipboard.writeText` sur le clic) : aucune
  commande ni capacité de l'appli. Le texte copié est affiché ; si le presse-papiers refuse, il reste affiché pour
  une copie à la main.
- **Commandes** (`app/src/interface.rs`, câblage ; logique dans `mcp_proxy_gateway::clients_view`) :
  `get_llm_clients`, `preview_llm_client(location, action)`, `apply_llm_client(location, action, fingerprint)`.
  `location` parmi `claude-desktop`, `claude-desktop-msix`, `vscode`, `action` parmi `install`, `remove` ;
  tout autre texte est refusé. La page ne donne jamais de chemin : `%APPDATA%` et `%LOCALAPPDATA%` sont lus par
  l'appli au démarrage. Déclarées dans `build.rs`, données par la seule capacité `main`, fenêtre appelante
  revérifiée. Réponses figées par `ui/src/api/clients.example.json` (contrat, comme l'instantané).
- **Journal** : « MCP Proxy ajouté à la configuration de Claude Desktop par l'utilisateur », « … retiré … », « …
  changée depuis l'aperçu : rien n'a été écrit », « … non modifiée : <raison> ». Jamais un chemin ni un contenu de
  fichier.

### Adaptateurs

Un adaptateur par client, isolé, pour qu'un changement de format chez un éditeur se corrige sans toucher au reste.
**Revérifier les chemins et formats dans la documentation de chaque client au moment de coder.**

| Client | Fichier | Entrée |
|---|---|---|
| Claude Desktop | `%APPDATA%\Claude\claude_desktop_config.json` | `mcpServers."mcp-proxy" = { "command": "<exe installé>", "args": ["stdio"] }` |
| VS Code | `%APPDATA%\Code\User\mcp.json` | `servers."mcp-proxy" = { "type": "stdio", "command": "<exe installé>", "args": ["stdio"] }` |
| Claude Desktop, paquet MSIX | `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude_desktop_config.json` | comme Claude Desktop |

Vérifié le 2026-11-22 (recherche sur le web, la documentation des éditeurs n'étant pas joignable d'ici) :

- **VS Code**, documentation officielle : configuration utilisateur `mcp.json` du dossier de profil (« MCP: Open
  User Configuration »), serveurs sous `servers`, serveur local `"type": "stdio"`, `command`, `args` ; un profil
  peut avoir sa propre configuration. Donné par des guides tiers seulement : le chemin
  `%APPDATA%\Code\User\mcp.json` (profil par défaut). Non pris en charge : les profils nommés, VS Code Insiders,
  le fichier `~/.copilot/mcp-config.json` d'un seul billet de blog.
- **Claude Desktop** : chemin documenté `%APPDATA%\Claude\claude_desktop_config.json`, `mcpServers`, `command`,
  `args`. Installé par son paquet MSIX, il lit le fichier du dossier virtualisé du paquet, alors que son bouton
  « Edit Config » ouvre celui de `%APPDATA%` (ticket `anthropics/claude-code#26073`, ouvert depuis février 2026,
  et plusieurs guides ; non documenté). Le nom de famille du paquet (`Claude_pzs8sxrjxfjjc`) vient de ces
  signalements : hypothèse à vérifier en recette (`EX-023-17`).
- **Deux emplacements de Claude Desktop** : chacun est une ligne du dialogue, avec son état et ses actions, chacun
  après son propre aperçu. Quand les deux existent, le dialogue dit que l'installation par paquet lit celui du
  paquet, et marque celui de `%APPDATA%` « sans doute pas lu » ; il n'annonce jamais « configuré » pour l'un en
  laissant croire que l'autre est configuré. Option retenue (I023.3) : ne rien décider à la place de l'utilisateur,
  puisqu'on ne sait pas lequel son installation lit ; configurer les deux est sans risque.
- Dossier du client : `%APPDATA%\Claude`, le dossier `…\LocalCache\Roaming\Claude` du paquet (créé par Claude
  Desktop à son premier démarrage), `%APPDATA%\Code\User`.

### Écriture sûre

- **Aperçu avant écriture** : le dialogue montre la différence, l'utilisateur valide.
- Sauvegarde `<fichier>.bak-<horodatage>` avant toute modification.
- Lecture tolérante au JSON avec commentaires et virgules finales. Le reste du fichier est conservé ; seule
  l'entrée `mcp-proxy` est touchée, jamais celle d'un autre serveur.
- Si le fichier ne peut pas être lu de façon sûre : **ne rien écrire**, afficher l'extrait à coller à la main.
- Écriture atomique (fichier temporaire puis renommage). Opération idempotente.
- Fichier absent : le créer seulement si le dossier du client existe.

Réalisation (amendement du 2026-11-23) :

- **Lecture** : `jsonc-parser` 0.34 (dprint), pour les positions seulement ; commentaires et virgules finales
  acceptés, toute autre extension (guillemets simples, clés sans guillemets, hexadécimal…) refusée ; marque d'ordre
  des octets conservée ; 1 Mio au plus ; UTF-8 seulement ; un lien ou un dossier à la place du fichier n'est ni lu
  ni remplacé. Sont aussi « illisibles » : racine qui n'est pas un objet, fichier fait de commentaires seulement,
  section des serveurs qui n'est pas un objet, section ou entrée `mcp-proxy` présente deux fois.
- **Modification** : quelques coupes et insertions dans le texte d'origine, jamais une réécriture. Nouvelle entrée
  à la fin de la section, avec l'indentation (espaces ou tabulations), les fins de ligne (CRLF ou LF) et le style de
  virgule finale du fichier ; un fichier sur une ligne reste sur une ligne ; une entrée existante est remplacée en
  place ; un commentaire qui suit le dernier serveur reste avec lui. Retrait : l'entrée et sa virgule, ou la virgule
  qui la précède si elle était la dernière ; une section qui ne contenait qu'elle redevient `{}` ; un commentaire de
  l'utilisateur hors de l'entrée n'est jamais retiré (sinon : refus). **Chaque résultat est relu** : même valeur que
  l'original à l'entrée `mcp-proxy` près, mêmes commentaires dans le même ordre hors de l'entrée ; la moindre
  différence refuse la modification (« la modification n'a pas pu être vérifiée ») et l'extrait est proposé.
- **Écriture** : sauvegarde `<fichier>.bak-<AAAAMMJJ-HHMMSS>` (UTC), créée sans jamais écraser (`-2` à `-9` si le
  nom est pris) ; texte écrit dans `<fichier>.mcp-proxy-<aléa>.tmp` du même dossier et vidé sur disque ; dernière
  lecture du fichier, qui doit être exactement celle de l'aperçu ; renommage sur le fichier. Un échec à n'importe
  quelle étape laisse le fichier intact et retire la sauvegarde et le fichier temporaire. « Déjà comme demandé » :
  rien n'est écrit, aucune sauvegarde.

### Organisation du code

- `mcp_proxy_deploy::clients` : détection, calcul de la modification, écriture. Chemins injectés, donc testable
  sous Linux.

Réalisation : `mcp_proxy_deploy::clients` (`Environment`, `LocationId`, `inspect`, `inspect_all`, `plan`, `apply`,
`Plan`, `Change`, `State`, `Unreadable`, `ClientError`, `generic_snippet` ; trait `ClientFiles` et `RealFiles` pour
les fichiers), un adaptateur par client (`clients/claude.rs`, `clients/vscode.rs` : section, entrée, dossiers),
l'éditeur tolérant (`clients/jsonc.rs`) et l'écriture sûre (`clients/files.rs`). Pour la désinstallation (spec 022) :
`configured` (emplacements qui ont une entrée `mcp-proxy`, la nôtre ou non) et `remove_entry` (retrait sans aperçu,
mêmes garanties). Vue et commandes : `mcp_proxy_gateway::clients_view`.

## Tests

Avec des fichiers d'exemple :

- **EX-023-01** — fichier absent, vide, avec d'autres serveurs, avec une entrée `mcp-proxy` ancienne, avec commentaires et virgules
  finales ;
- **EX-023-02** — les autres entrées et les autres clés sont intactes après installation et après retrait ;
- **EX-023-03** — fichier illisible : rien n'est écrit, l'extrait manuel est proposé ;
- **EX-023-04** — sauvegarde créée ; deux installations de suite donnent le même fichier ;
- **EX-023-05** — détection de chaque état.
- **EX-023-09** — Ce qui est écrit est ce qui a été montré : un fichier réécrit entre l'aperçu et l'écriture (y
  compris après la dernière lecture), une autre action, un autre emplacement ou un autre exe que ceux de l'aperçu
  sont refusés, sans rien écrire ni laisser.
- **EX-023-10** — Un échec de la sauvegarde, du fichier temporaire ou du renommage laisse le fichier intact, sans
  sauvegarde ni fichier temporaire, avec un message sans chemin.
- **EX-023-11** — Client non installé : rien n'est créé ; MCP Proxy non installé : rien n'est ajouté, le retrait
  reste possible.
- **EX-023-12** — Les deux emplacements de Claude Desktop sont vus et modifiés séparément ; `configured` et
  `remove_entry` (spec 022) retirent chaque entrée avec sa sauvegarde.
- **EX-023-13** — Rien des fichiers (valeurs d'autres serveurs, noms de serveurs, chemins de l'utilisateur) dans un
  `Debug`, une erreur, le journal ni les logs ; la fenêtre ne nomme qu'un emplacement et une action connus.
- **EX-023-14** — Contrat de la fenêtre : vues, aperçus et issues sérialisés sont `ui/src/api/clients.example.json`,
  lu strictement par `types.ts` ; commandes sur l'assemblage réel (aperçu, refus d'un fichier changé, écriture,
  journal, identifiants inconnus refusés).
- **EX-023-15** — Vitest : menu de l'en-tête, états, boutons offerts, aperçu puis envoi de l'empreinte, nouvel
  aperçu après un refus, extrait à coller, copie (et refus du presse-papiers), message de redémarrage, erreur en
  rouge dans le dialogue.
- **EX-023-16** — Playwright : captures du menu et du dialogue en clair et en sombre (états, aperçu, après
  installation, fichier illisible, deux emplacements, MCP Proxy non installé, fichier changé) ; aucune erreur de
  console ni de CSP ; le dialogue tient dans la fenêtre ; orange seulement pour « configuré avec un autre chemin »,
  rouge seulement pour l'illisible ; la configuration copiée est lue dans le presse-papiers.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-023-06** (manuel) — « Installer » pour Claude Desktop, redémarrer Claude Desktop : les outils de MCP Proxy apparaissent.
- **EX-023-07** (manuel) — Idem pour VS Code.
- **EX-023-08** (manuel) — « Retirer » enlève l'entrée et laisse les autres serveurs en place.
- **EX-023-17** (manuel) — Claude Desktop installé par son paquet MSIX : quel fichier il lit réellement, le nom du
  dossier du paquet, ce que montre le dialogue quand les deux fichiers existent, et si Claude Desktop réécrit ou vide
  son fichier après un redémarrage ou une mise à jour.
- **EX-023-18** (manuel) — Le dialogue dans l'appli : menu, copie dans le presse-papiers de la WebView, fichier
  avec commentaires et CRLF conservé, client en cours qui réécrit son fichier entre l'aperçu et l'écriture, appli
  lancée avec `--no-install`.

## Amendements

- **2026-11-23** — Implémentation. Écarts au texte : Claude Desktop a **deux emplacements** (installation
  classique et paquet MSIX, dont le fichier est le seul lu par cette installation, d'après un ticket encore ouvert
  et des guides : non documenté), chacun une ligne du dialogue, ajoutés au tableau des adaptateurs ; cinquième
  état « fichier illisible » ; « Copier la configuration » et « Installer » sont indisponibles tant que MCP Proxy
  n'est pas installé (l'exe à écrire n'existe pas) ; « Installer » sur « configuré avec un autre chemin » remplace
  l'entrée après aperçu ; ce qui est écrit doit être ce que l'aperçu a montré (empreinte), sinon nouvel aperçu ;
  VS Code : profil par défaut seulement. Ajouts : précisions de réalisation (interface, lecture, modification,
  écriture, organisation du code, commandes, journal) et ce que la spec 022 utilisera. Chemins et formats revérifiés
  le 2026-11-22 (section « Adaptateurs »). Critères ajoutés : `EX-023-09` à `EX-023-16` (automatiques), `EX-023-17`
  et `EX-023-18` (recette). Aucun critère retiré.
- **2026-11-23** — Libellé de la variante MSIX de Claude Desktop : « paquet MSIX (installeur officiel) » au lieu de
  « paquet MSIX (Microsoft Store) ». Raison : la source (ticket `anthropics/claude-code#26073`) dit que l'installeur
  officiel est un paquet MSIX ; elle ne parle pas du Microsoft Store, et le mot aurait fait croire à une personne
  passée par le site que la remarque ne la concerne pas. Aucun critère touché.
