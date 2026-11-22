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
  login à la spec 012 ; depuis, la fenêtre de connexion s'ouvre : voir la section 012).
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

## 012 — app — fenêtres de login WebView2 (JWT, cookie, Basic), profil InPrivate

Préparation : comme pour la 011 (`cd ui && npm ci && npm run build`, puis compiler). Version du runtime :
`reg query "HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv` (ou
la même clé sous `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients`). Le dossier du profil est
`%LOCALAPPDATA%\MCP Proxy\webview\` ; celui de la sonde, `%LOCALAPPDATA%\MCP Proxy\webview-probe\`. Pour les
critères 04 à 07, 16 et 17, il faut une config dont les sessions pointent vers un vrai SSO de recette (les hôtes de
la config embarquée sont fictifs) : la compiler dans l'exe (`config/default.toml`), sans la commiter.

- [ ] **EX-012-11** — `cargo run -p mcp-proxy --example webview_probe` (ne pas toucher les fenêtres, environ
  1 min 30) : 8 lignes « RÉUSSI » et « Bilan : 8/8 RÉUSSI ». Noter le résultat des points 3 (« InPrivate effectif »)
  et 4 (« SSO partagé »), non attestés par les sources : s'ils échouent, le signaler et passer la config d'exemple
  en `profile = "ephemeral_disk"`. Contrôle du détecteur :
  `cargo run -p mcp-proxy --example webview_probe -- --profil ephemeral_disk` doit donner le point 3 en ÉCHEC avec
  « cookie FAKE_SSO_SESSION trouvé … : détecteur vérifié » (si le cookie n'est pas trouvé, relancer avec
  `--attente-cookies 90` et le noter). Relancer aussi avec `--sso-windows` : mêmes résultats en `in_memory`.
- [ ] **EX-012-04** — Appli lancée, client LLM configuré (EX-011-06), session jamais connectée : appeler un outil de
  la session. Une fenêtre « Connexion — {libellé} » s'ouvre au premier plan sur la page de login ; se connecter :
  la fenêtre se ferme seule, l'appel aboutit, et la carte de la session (ou l'infobulle) passe à « active jusqu'à
  HH:MM ». Le journal du jour ne contient ni le jeton ni la valeur du cookie (chercher un fragment de la valeur
  copiée depuis les outils du SSO, pas depuis la fenêtre de l'appli).
- [ ] **EX-012-05** — Même appel, session expirée ou déconnectée : fermer la fenêtre de connexion par la croix sans
  se connecter. Le client LLM affiche « connexion à la session « … » annulée par l'utilisateur » ; l'icône quitte
  l'état « attention ».
- [ ] **EX-012-06** — Deux sessions d'un même fournisseur (par exemple `sso-corp` puis une session `cookie` sur le
  même SSO) : se connecter à la première, puis appeler un outil de la seconde sans quitter l'appli. La seconde
  fenêtre aboutit sans ressaisie (SSO déjà ouvert), ou se ferme seule. Quitter puis relancer l'appli : la
  connexion est redemandée (rien n'a survécu sur le disque).
- [ ] **EX-012-07** — Session active : « Déconnecter » (spec 016 ; en attendant, attendre l'expiration ou relancer
  l'appli), puis « Se connecter » ou un appel : les identifiants sont redemandés par la page de login.
- [ ] **EX-012-16** — Session `basic` (`sso-tools` avec une `probe_url` réelle) : un appel ouvre la petite fenêtre
  « Connexion — SSO Outils » au premier plan, champs vides. Un mauvais mot de passe : la fenêtre se rouvre avec
  « identifiants refusés », l'identifiant prérempli, le mot de passe vide. « Annuler » : le LLM reçoit
  « connexion … annulée par l'utilisateur » ; la croix : idem. Un bon mot de passe : l'appel aboutit. Le journal
  ne contient pas le mot de passe.
- [ ] **EX-012-17** — Dans une fenêtre de login ouverte : F12, Ctrl+Maj+I et Ctrl+Maj+J n'ouvrent rien ; le clic
  droit n'offre pas « Inspecter » ; un lien `target=_blank` de la page du SSO, s'il y en a, n'ouvre aucune
  fenêtre ; un téléchargement proposé par la page ne démarre pas ; le titre reste « Connexion — {libellé} » quelle
  que soit la page ; le journal contient « nouvelle fenêtre refusée » ou « téléchargement refusé » selon le cas.
  Observer aussi que la page de login du SSO fonctionne normalement (le prototype `Object` y est figé par Tauri,
  I012.17) et le noter si ce n'est pas le cas.

## 013 — app — notifications Windows et confirmations

Préparation : comme pour la 011 (`cd ui && npm ci && npm run build`, puis compiler), profil « Développeur »
(EX-011-06) avec PROD exposé (ajouter `"prod"` à `exposed_environments`). `prod_jira_add_comment` (écriture en PROD)
demande une confirmation par notification ; tant que `sso-tools` n'a pas de `probe_url` réelle, l'appel autorisé
échoue ensuite à la connexion, ce qui suffit à distinguer « autorisé » (« connexion … ») de « refusé » (« appel refusé
par l'utilisateur »). Notifications de Windows activées, « Ne pas déranger » désactivé, sauf mention contraire. Le
journal du jour dit pourquoi une fenêtre remplace un toast (« notifications indisponibles : … »).

- [ ] **EX-013-10** — À faire en premier. Copier l'exe dans un dossier quelconque (par exemple `C:\Temp\mcp`), sans
  raccourci dans le menu Démarrer, et le lancer. `reg query "HKCU\Software\Classes\AppUserModelId\McpProxy.Desktop"`
  donne `DisplayName` « MCP Proxy » et `IconUri` vers `%LOCALAPPDATA%\MCP Proxy\notification-icon.png` (fichier
  présent). Déclencher une confirmation (EX-013-05) : le toast porte le nom « MCP Proxy » et l'icône de l'appli. Si
  aucun toast n'apparaît alors que le journal ne dit pas « notifications indisponibles », le noter : l'AUMID du
  registre ne suffirait pas sur ce poste (non attesté par les sources).
- [ ] **EX-013-05** — Appeler `prod_jira_add_comment` avec `{"key":"DEMO-1","body":"texte"}`. Un toast « Confirmation :
  PROD / jira / add_comment » montre « Demandé par {client} » et les arguments, avec « Refuser » et « Autoriser une
  fois », et reste affiché. « Autoriser une fois » : le LLM ne reçoit pas « appel refusé par l'utilisateur » (l'appel
  passe à l'étape suivante). Recommencer, « Refuser » : le LLM reçoit « appel refusé par l'utilisateur. ». Si un
  clic sur un bouton ne produit rien (le toast disparaît, l'appel attend puis expire), le noter : les clics ne
  seraient pas remis au processus avec le seul AUMID du registre.
- [ ] **EX-013-06** — Même appel avec un `body` de 300 caractères. Le toast propose « Refuser » et « Voir le détail »,
  pas « Autoriser une fois ». « Voir le détail » : une fenêtre « Confirmation — PROD / jira / add_comment » s'ouvre
  devant les autres, montre chemin, client, politique, les arguments complets et le compte à rebours ; la fermer par
  la croix : « appel refusé par l'utilisateur. ». Recommencer et cliquer « Autoriser une fois » dans la fenêtre :
  l'appel passe ; la fenêtre se ferme seule.
- [ ] **EX-013-07** — Déclencher une confirmation et ne rien toucher pendant 30 s : le LLM reçoit « confirmation non
  reçue dans le délai (30 s), appel refusé. » ; le toast disparaît de l'écran **et** du centre de notifications
  (Win+N). Avec la fenêtre de détail ouverte : elle se ferme seule à l'échéance.
- [ ] **EX-013-08** — Activer « Ne pas déranger » (Paramètres, Système, Notifications). Déclencher une confirmation :
  aucun toast, la fenêtre de détail s'ouvre seule au premier plan ; le journal contient « notifications
  indisponibles : mode « Ne pas déranger » ». Même chose après avoir désactivé les notifications de « MCP Proxy »
  seulement. Réactiver ensuite.
- [ ] **EX-013-09** — Pendant une confirmation en attente, l'icône est orange et son infobulle dit « 1 confirmation
  attend » ; après la réponse (ou l'échéance), elle redevient neutre (« MCP Proxy — prêt »).
- [ ] **EX-013-04** — Session `sso-corp` jamais connectée : appeler un outil de `gitlab-ops`. Un toast « Connexion
  demandée » dit « {client} attend une connexion à SSO Entreprise » ; mettre une autre fenêtre devant la fenêtre de
  connexion, puis cliquer le toast (ou son entrée dans Win+N) : la fenêtre de connexion revient devant. Idem avec une
  session Basic (`sso-tools`) : la fenêtre de saisie revient devant.
- [ ] **EX-013-15** — Toast de confirmation : le balayer (croix ou glissement) : « appel refusé par l'utilisateur. » ;
  recommencer et cliquer sur le corps du toast (pas un bouton) : la fenêtre de détail s'ouvre, rien n'est approuvé.
  Laisser un toast filer dans le centre de notifications sans répondre : après l'échéance, il n'y est plus. Quitter
  l'appli avec une confirmation en attente : le LLM ne reçoit aucune autorisation (refus ou serveur fermé) et Win+N
  ne garde aucun toast de l'appli. Information : modifier la liste d'outils d'un MCP connecté (ou, faute de MCP réel,
  écrire `{ pas du json` dans `settings.json` puis relancer : toast « Réglages réinitialisés ») ; son clic ouvre la
  fenêtre principale.

## 025 — Module MCP local d'exemple (`jira-local`)

À dérouler quand l'appli (spec 011) et les fenêtres de saisie Basic (spec 012) existent, avec une config dont
l'instance `jira` pointe vers un Jira réel et dont `sso-tools` a une `probe_url` sur ce Jira.

- [ ] **EX-025-08** — Depuis un client LLM, rechercher des tickets (`rec_jira_search_issues`) : la saisie Basic de
  « SSO Outils » s'ouvre, puis la réponse liste les tickets attendus, sans adresse e-mail ni lien `self`. Récupérer
  un ticket (`prod_jira_get_issue`). Ajouter un commentaire en PROD (`prod_jira_add_comment`) : la confirmation
  s'affiche ; un refus n'ajoute rien dans Jira, une acceptation ajoute le commentaire. Changer le mot de passe dans
  Jira puis relancer une recherche : une nouvelle saisie est demandée, une seule, et la recherche aboutit.

