# Recette manuelle sous Windows

Vérifications qui ne peuvent pas être exécutées dans le conteneur Linux (le code Windows y est seulement compilé).
À dérouler sur un poste Windows avec la chaîne Rust du projet.

Une section par spec qui a des critères `(manuel)`, une case par critère. Chaque ligne cite l'identifiant du
critère : `cargo xtask trace` vérifie que tous les critères manuels d'une spec implémentée figurent ici.

## 002 — core — store de secrets en mémoire

- [ ] **EX-002-09** — Aucun message « secrets en mémoire NON chiffrés » dans la sortie : le protecteur de
  développement n'existe pas sous Windows.
- [ ] **EX-002-16** — `cargo test -p mcp-proxy-core platform::windows` : les 2 tests passent (aller-retour DPAPI
  utilisateur réel par WinRT ; données protégées différentes des données claires et sans la valeur en clair ;
  données quelconques refusées ou rendues vides).
- [ ] **EX-002-17** — `cargo test -p mcp-proxy-core secrets` : tous les tests passent. `platform_store_round_trip`
  y utilise `DpapiKeyWrapper`, aller-retour d'essai de `SecretStore::for_platform` compris ; les tests du client HTTP
  passent par SChannel (requêtes `http` en boucle locale).

## 011 — app — squelette Tauri, pont stdio (named pipe), instance unique, tray

Préparation : sur le poste Windows, `cd ui && npm ci && npm run build`, puis `cargo build -p mcp-proxy --release` ;
l'exe est `target\release\mcp-proxy.exe`. Le journal est dans `%LOCALAPPDATA%\MCP Proxy\logs\`. Pour repartir
d'un premier lancement : quitter l'appli et supprimer `%LOCALAPPDATA%\MCP Proxy\settings.json`. Vérifier d'abord
que l'exe porte son icône dans l'Explorateur (ressources compilées seulement sous Windows, I011.14).

- [ ] **EX-011-04** — Lancer `mcp-proxy.exe` (double clic) : aucune fenêtre de console ; l'icône apparaît dans la
  zone de notification. Clic droit, « Ouvrir » : la fenêtre « MCP Proxy » s'affiche ; clic gauche sur l'icône :
  idem. Fermer la fenêtre par la croix : elle disparaît, l'icône reste, l'appli tourne (Gestionnaire des tâches).
- [ ] **EX-011-05** — L'appli tournant, fenêtre cachée, relancer `mcp-proxy.exe` : aucun second processus ne reste
  (Gestionnaire des tâches), aucune boîte « canal occupé », la fenêtre de la première instance passe au premier
  plan.
- [ ] **EX-011-06** — Configurer un client MCP (Claude Desktop ou VS Code) avec la commande
  `C:\…\mcp-proxy.exe` et l'argument `stdio`. Choisir le profil « Développeur » : tant que l'interface de la
  spec 016 n'existe pas, appli arrêtée, écrire dans `settings.json` :
  `{"schema_version":1,"profile":"developpeur","exposed_environments":["dev","int","rec"],"disabled_mcp":[],`
  `"hardening":{},"autostart":false,"beta_channel":false,"gateway_suspended":false}` (sur une ligne). Le client liste les outils du
  catalogue des environnements DEV, INT et REC (par exemple `rec_jira_search_issues`). Appeler un outil qui demande
  une connexion : le client affiche « connexion à « … » impossible pour le moment, appel refusé. » (fenêtres de
  login à la spec 012).
- [ ] **EX-011-07** — Quitter l'appli (« Quitter »). Relancer le client MCP : `mcp-proxy.exe stdio` démarre
  l'appli (l'icône réapparaît en moins de 15 s) et les outils sont listés. Fermer le client : l'appli reste lancée
  (si elle se ferme avec le client, le noter : le job du client interdit la sortie du job, I011.10).
- [ ] **EX-011-10** — Appli arrêtée, créer le pipe depuis PowerShell :
  `$p = New-Object System.IO.Pipes.NamedPipeServerStream("mcp-proxy.<domaine>.<utilisateur>")` (nom en
  minuscules, voir `USERDOMAIN` et `USERNAME`), puis lancer `mcp-proxy.exe` : boîte « Un autre programme occupe le
  canal de MCP Proxy… », l'appli s'arrête, aucune icône ne reste ; le journal du jour contient « démarrage refusé ».
  `$p.Dispose()` puis relancer : l'appli démarre.
- [ ] **EX-011-11** — Avec un second compte Windows (changement rapide d'utilisateur ou `runas /user:autre
  powershell`), l'appli du premier utilisateur tournant : 1) ouvrir son pipe
  (`New-Object System.IO.Pipes.NamedPipeClientStream(".", "mcp-proxy.<domaine>.<premier>", "InOut")`, `Connect()`)
  puis écrire une requête MCP : la connexion est fermée sans réponse, le journal du premier utilisateur contient
  « connexion au canal refusée avant tout échange MCP » ; 2) le second compte crée un pipe au nom du premier
  utilisateur avant que celui-ci lance quoi que ce soit, puis le premier utilisateur lance `mcp-proxy.exe stdio`
  avec ce nom (variables `USERDOMAIN`/`USERNAME` identiques) : le pont écrit « le canal de MCP Proxy n'appartient
  pas à l'utilisateur courant » sur stderr, sort avec un code non nul, et le pipe du second compte ne reçoit aucun
  octet de la requête MCP.
- [ ] **EX-011-12** — Après un premier démarrage : `reg query "HKCU\Software\Microsoft\Windows\Windows Error
  Reporting\ExcludedApplications" /v mcp-proxy.exe` donne `REG_DWORD 0x1`. Relancer : la valeur est inchangée.
- [ ] **EX-011-17** — L'icône neutre se lit sur barre des tâches claire et sombre (Paramètres, Personnalisation,
  Couleurs). Sans `settings.json`, la fenêtre s'ouvre seule au lancement. Quitter, écrire `{ pas du json` dans
  `settings.json`, relancer : l'icône est rouge et son infobulle dit « MCP Proxy — 1 erreur ».
- [ ] **EX-011-18** — Un client MCP connecté, « Quitter » : l'appli s'arrête, le client voit le serveur fermé
  (le pont se termine). Le journal du jour contient « démarrage de MCP Proxy », « client connecté au canal » et
  « arrêt de MCP Proxy », et aucune valeur de jeton, cookie ou mot de passe.

## 025 — Module MCP local d'exemple (`jira-local`)

À dérouler quand l'appli (spec 011) et les fenêtres de saisie Basic (spec 012) existent, avec une config dont
l'instance `jira` pointe vers un Jira réel et dont `sso-tools` a une `probe_url` sur ce Jira.

- [ ] **EX-025-08** — Depuis un client LLM, rechercher des tickets (`rec_jira_search_issues`) : la saisie Basic de
  « SSO Outils » s'ouvre, puis la réponse liste les tickets attendus, sans adresse e-mail ni lien `self`. Récupérer
  un ticket (`prod_jira_get_issue`). Ajouter un commentaire en PROD (`prod_jira_add_comment`) : la confirmation
  s'affiche ; un refus n'ajoute rien dans Jira, une acceptation ajoute le commentaire. Changer le mot de passe dans
  Jira puis relancer une recherche : une nouvelle saisie est demandée, une seule, et la recherche aboutit.

