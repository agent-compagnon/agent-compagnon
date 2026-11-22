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
d'un premier lancement : quitter l'appli et supprimer `%LOCALAPPDATA%\MCP Proxy\settings.json`. Depuis la spec 014,
les réglages sont scellés : un `settings.json` écrit à la main est refusé ; depuis la spec 016, le profil se choisit
dans la fenêtre (dialogue du premier lancement, puis sélecteur de l'en-tête). Vérifier d'abord
que l'exe porte son icône dans l'Explorateur (ressources compilées seulement sous Windows, I011.14). Depuis la spec
019, un exe lancé hors de `%LOCALAPPDATA%\Programs\MCP Proxy\` propose son installation : pour les sections 011 à
018, l'installer d'abord (section 019) et lancer l'exe installé, ou lancer `target\release\mcp-proxy.exe
--no-install`.

- [ ] **EX-011-04** — Lancer `mcp-proxy.exe` (double clic) : aucune fenêtre de console ; l'icône apparaît dans la
  zone de notification. Clic droit, « Ouvrir » : la fenêtre « MCP Proxy » s'affiche ; clic gauche sur l'icône :
  idem. Fermer la fenêtre par la croix : elle disparaît, l'icône reste, l'appli tourne (Gestionnaire des tâches).
- [ ] **EX-011-05** — L'appli tournant, fenêtre cachée, relancer `mcp-proxy.exe` : aucun second processus ne reste
  (Gestionnaire des tâches), aucune boîte « canal occupé », la fenêtre de la première instance passe au premier
  plan.
- [ ] **EX-011-06** — Configurer un client MCP (Claude Desktop ou VS Code) avec la commande
  `C:\…\mcp-proxy.exe` et l'argument `stdio`. Choisir le profil « Développeur » dans la fenêtre (dialogue du
  premier lancement ou sélecteur de l'en-tête, spec 016 ; `settings.json` écrit à la main est refusé depuis la
  spec 014). Le client liste les outils du
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

Préparation : comme pour la 011 (`cd ui && npm ci && npm run build`, puis compiler), profil « Support » choisi dans la
fenêtre (sélecteur de l'en-tête, spec 016 ; il expose REC, PPROD et PROD). `prod_jira_add_comment` (écriture en PROD)
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
  écrire `{ pas du json` dans `settings.json` puis relancer : toast « Réglages réinitialisés », puis rechoisir le
  profil par le menu) ; son clic ouvre la
  fenêtre principale.

## 014 — Réglages scellés (HMAC + DPAPI)

Préparation : comme pour la 011. Les fichiers sont dans `%LOCALAPPDATA%\MCP Proxy\` : `settings.json` commence par
une ligne `MCP-PROXY-SEAL v1 hmac-sha256=…`, suivie du JSON des réglages ; `settings.key` porte la clé enveloppée
par DPAPI. Avant tout, `cargo test -p mcp-proxy-core sealed` sur le poste Windows : tous les tests passent, dont
les deux du module `dpapi` (critère automatique `EX-014-04`, compilé seulement sous Linux).

- [ ] **EX-014-09** — À faire en premier, avec un `settings.json` de la version précédente (sans ligne
  `MCP-PROXY-SEAL`), par exemple
  `{"schema_version":1,"profile":"developpeur","exposed_environments":["dev","int","rec"],"disabled_mcp":[],`
  `"hardening":{},"autostart":false,"beta_channel":false,"gateway_suspended":false}` sur une ligne. Lancer la
  nouvelle version : la notification « Réglages réinitialisés » s'affiche, l'icône est rouge (« 1 erreur »), aucun
  outil n'est listé par le client LLM, le journal du jour contient une ligne `mcp.event` « réglages remplacés par
  les plus stricts : réglages refusés : fichier sans sceau … ». La fenêtre s'ouvre sur le dialogue du choix du
  profil, au-dessus du bandeau rouge « Vos réglages n'ont pas pu être vérifiés… ». Choisir « Développeur »,
  « Continuer » : le bandeau disparaît, l'en-tête dit « Développeur », le client liste les outils de DEV, INT et
  REC. `settings.json` commence alors par `MCP-PROXY-SEAL`, et `settings.key` existe. Quitter, relancer : toujours
  « Développeur », aucun bandeau ni notification.
- [ ] **EX-014-06** — Profil « Support » choisi par le sélecteur de l'en-tête (le dialogue cite « REC, PPROD,
  PROD », sensibles « PPROD, PROD » ; « Annuler » ne change rien, « Changer de profil » applique), quitter,
  relancer : l'en-tête dit « Support », aucune alerte, aucune notification, le client liste les outils de REC,
  PPROD et PROD.
- [ ] **EX-014-05** — Appli arrêtée, ouvrir `settings.json` dans le Bloc-notes, remplacer `"support"` par
  `"developpeur"` (ou ajouter `"dev"` à `exposed_environments`), enregistrer, relancer : profil actif « Lecture
  seule » (la fenêtre redemande le profil), aucun environnement exposé (le client ne liste aucun outil),
  notification « Réglages réinitialisés », bandeau d'erreur et icône rouge ; le journal dit « sceau invalide ».
  Même chose en supprimant seulement la première ligne du fichier, ou en supprimant `settings.key`.
- [ ] **EX-014-07** — Ouvrir `settings.key` dans un éditeur hexadécimal (ou `Format-Hex`) : après l'en-tête
  `MCP-PROXY-SEAL-KEY v1`, plusieurs centaines d'octets (enveloppe DPAPI), et non 32. Copier `settings.json` et
  `settings.key` dans `%LOCALAPPDATA%\MCP Proxy\` d'un autre compte Windows, y lancer l'appli : réglages stricts,
  notification « Réglages réinitialisés », journal « clé de scellement illisible ». Vérifier aussi qu'un cache de
  catalogue copié avec eux (`cache\catalog\*.json`) est ignoré (alerte d'information « scellement invalide »).

## 015 — Arrêt d'urgence dans le menu de l'icône

Préparation : comme pour la 011, profil « Développeur » choisi dans la fenêtre, client LLM configuré
(EX-011-06), et pour EX-015-08 une config dont `sso-dev` (ou une autre session `cookie`/`jwt_header`) pointe vers un
vrai SSO de recette (section 012). Le menu de l'icône contient, dans l'ordre : « Ouvrir »,
« Suspendre la passerelle », « Tout déconnecter », « Quitter » (plus de « Profil » depuis la spec 016).

- [ ] **EX-015-06** — Menu de l'icône, « Suspendre la passerelle » (aucune question n'est posée) : l'icône devient
  grise avec une pause, son infobulle commence par « MCP Proxy — passerelle suspendue », l'élément du menu devient
  « Reprendre ». Le client liste toujours les mêmes outils ; tout appel, y compris une lecture en DEV, répond
  « MCP Proxy : passerelle suspendue par l'utilisateur. » ; le journal du jour contient une ligne `mcp.event`
  « passerelle suspendue par l'utilisateur » et des appels au statut `suspended`. Avec une confirmation en attente
  (profil « Support », `prod_jira_add_comment`, section 013) puis « Suspendre » : le toast ou la fenêtre de détail
  disparaît et le client reçoit aussitôt le message de suspension. « Reprendre » : l'icône redevient neutre, les
  appels passent de nouveau.
- [ ] **EX-015-07** — Suspendre, « Quitter », relancer l'appli : l'icône est grise, le menu dit « Reprendre » et les
  appels sont refusés. « Reprendre », quitter, relancer : l'icône est neutre.
- [ ] **EX-015-08** — Se connecter à une session (EX-012-04), puis lancer un appel d'une autre session et laisser sa
  fenêtre de connexion ouverte. Menu, « Tout déconnecter » (aucune question) : la fenêtre de connexion se ferme, le
  client reçoit « connexion à « … » interrompue, l'utilisateur a déconnecté toutes les sessions ; appel refusé… », le
  journal contient « toutes les sessions déconnectées par l'utilisateur ». Rappeler un outil de la première
  session : la fenêtre de connexion s'ouvre et la page du SSO redemande les identifiants (aucune session SSO n'a
  survécu). Dans la fenêtre, toutes les cartes de l'onglet Connexions disent « jamais connectée ». Noter si une boîte « les données des fenêtres de connexion n'ont pas pu être effacées »
  apparaît (runtime WebView2 trop ancien pour `ClearBrowsingDataAll`, I015.4).

## 016 — ui — socle Vue/shadcn-vue, en-tête et profil, onglet Connexions

Préparation : comme pour la 011 (`cd ui && npm ci && npm run build`, puis compiler), client LLM configuré
(EX-011-06), et pour EX-016-05 une config dont `sso-dev` (ou une autre session `cookie`/`jwt_header`) pointe vers un
vrai SSO de recette (section 012). Pour repartir d'un premier lancement : quitter l'appli, supprimer
`%LOCALAPPDATA%\MCP Proxy\settings.json`.

- [ ] **EX-016-04** — Premier lancement : la fenêtre s'ouvre seule sur « Choisissez votre profil » ; Échap et un clic
  à côté ne la ferment pas ; « Continuer » reste inactif tant qu'aucun profil n'est coché. Choisir « Développeur »,
  « Continuer » : l'onglet Connexions s'affiche, l'en-tête dit « Profil Développeur », la section « SSO par
  environnement » montre SSO DEV, SSO INT et SSO REC, toutes « jamais connectée » (point en contour).
- [ ] **EX-016-05** — « Se connecter » sur SSO DEV : la fenêtre de connexion s'ouvre, la carte passe à « connexion
  demandée » (point orange, sans bandeau : aucun client n'attend). Recliquer « Se connecter » ramène la fenêtre
  devant. Se connecter : la carte dit « active jusqu'à HH:MM » à l'heure locale de Windows (et non UTC), point
  plein, bouton « Déconnecter ». « Déconnecter » : la carte dit « expirée », point atténué ; un appel d'outil de
  DEV redemande une connexion et la page du SSO redemande les identifiants.
- [ ] **EX-016-09** — Appeler depuis le client LLM un outil de REC qui demande « SSO Outils » : un bandeau orange
  « Claude Desktop attend une connexion à SSO Outils » (nom du client tel qu'il se déclare) apparaît avec « Se
  connecter », qui ramène la saisie Basic devant ; il disparaît à la fin de la connexion. Profil « Support » par
  l'en-tête : le dialogue cite « REC, PPROD et PROD », sensibles « PPROD et PROD » ; « Annuler » ne change rien,
  « Changer de profil » applique. Appeler `prod_jira_add_comment` : bandeau orange « Une confirmation attend votre
  réponse. » ; « Voir » ouvre la fenêtre de détail (arguments complets) sans rien autoriser, et le bandeau reste
  jusqu'à la réponse donnée dans cette fenêtre. Menu de l'icône, « Suspendre la passerelle » : bandeau « La
  passerelle est suspendue… » ; « Reprendre » dans la fenêtre le retire et le menu de l'icône redevient
  « Suspendre la passerelle ». Le menu de l'icône n'a pas d'entrée « Profil ».
- [ ] **EX-016-10** — Paramètres de Windows, Couleurs, mode sombre puis clair : la fenêtre principale suit (fond,
  texte, cartes lisibles), la saisie Basic et la fenêtre de détail aussi. Ouvrir une compilation de débogage
  (outils de développement actifs) : la console ne montre aucune violation de CSP.

## 017 — ui — onglet MCP

Appli compilée sous Windows, profil « Développeur » (DEV, INT et REC exposés), un client LLM branché sur le pont
(Claude Desktop ou VS Code), et pour la liste réelle au moins une instance joignable.

- [ ] **EX-017-08** — Onglet MCP, pastille INT : elle passe en contour, le compteur d'outils baisse ; recharger la
  liste d'outils côté client (VS Code : « MCP: List Servers », redémarrer le serveur ; Claude Desktop : nouvelle
  conversation) : aucun outil `int_…` n'y figure, et un appel à un ancien nom `int_…` est refusé (« inconnu ou non
  exposé »). Pastille PROD : un dialogue demande « Exposer PROD aux clients LLM ? » ; « Annuler » ne change rien,
  « Exposer » ajoute les outils `prod_…`, sous les verrous de la config (colonne Politique). Redémarrer l'appli :
  l'exposition est gardée ; le Journal (spec 018) ou le fichier de log montre « environnement « PROD » exposé par
  l'utilisateur ».
- [ ] **EX-017-09** — Interrupteur « Exposé » de Kibana Logs en DEV : il se coupe sur toutes les lignes de Kibana
  Logs (état « coupé »), et seuls les outils `*_kibana_*` disparaissent du client après rechargement de sa liste ;
  ceux de GitLab Ops et des autres MCP restent. Le rétablir les fait revenir. Sur une ligne de PRA (non exposé),
  l'interrupteur est grisé et son infobulle dit « PRA n'est pas exposé : exposez-le d'abord avec sa pastille. »
- [ ] **EX-017-12** — Dans la fenêtre de l'appli, en clair puis en sombre : infobulles (Politique, État, compteur,
  pastilles) et menu du filtre d'environnement s'affichent sans erreur de CSP dans la console (compilation de
  débogage) ; la recherche « kibána index » déplie les lignes de Kibana Logs et surligne « index » ; une instance
  injoignable montre « injoignable » en rouge et sa dernière erreur dans la ligne dépliée, « Recharger la liste »
  relance la lecture. Exposer tous les environnements : le compteur passe à l'orange au-delà de 100 outils.

## 018 — ui — onglet Journal

Préparation : l'appli de la spec 011, un client LLM (Claude Desktop ou VS Code) branché sur le pont, le profil
« Développeur » avec PROD exposé (onglet MCP), une session active en DEV. Ouvrir l'onglet Journal ; la console de
la fenêtre (compilation de débogage) doit rester sans erreur de CSP pendant toute la section. Un secret de test
connu (le mot de passe d'une session Basic, par exemple) sert à vérifier qu'il n'apparaît nulle part.

- [ ] **EX-018-07** — Depuis le client : un appel de lecture en DEV (`dev_kibana_search_logs`) apparaît aussitôt en
  tête avec « réussi » et sa durée, sans chevron ; un appel destructif refusé par la politique
  (`prod_k8s_delete_pod`) apparaît « interdit », en noir et blanc ; un appel qui dure plus de 5 s (requête large sur
  Kibana, ou serveur ralenti) apparaît avec la marque « lent » ; un appel dont les arguments contiennent le secret
  de test apparaît « bloqué ». Les filtres « Problèmes » (lent, bloqué) et « Politiques » (interdit) les
  retrouvent ; la recherche « kibána » trouve l'appel de Kibana Logs.
- [ ] **EX-018-08** — La ligne « bloqué » se déplie : trace, client, session, politique ; la requête montre
  `[SECRET MASQUÉ]` à la place du secret de test, qui n'apparaît ni dans la ligne, ni dans le détail, ni dans la
  recherche (le chercher ne trouve rien).
- [ ] **EX-018-09** — « Exporter un diagnostic », enregistrer : le message donne le chemin ; ouvrir l'archive avec
  l'Explorateur : `LISEZMOI.txt`, `version.txt`, `systeme.txt` (version de Windows et de WebView2),
  `config.txt`, `reglages.json`, `instances.txt` et `logs\mcp-proxy-*.log` ; rechercher le secret de test dans
  tous les fichiers (`findstr /s /i` sur le dossier extrait) : aucun résultat.
- [ ] **EX-018-13** — En clair puis en sombre (réglage de Windows) : les entrées arrivent en direct ; « En
  direct » passe à « En pause », la liste ne bouge plus et « 2 nouvelles entrées depuis la pause. » s'affiche après
  deux appels ; « Afficher » les met en tête. Les infobulles « Problèmes », « Politiques » et « lent », et le menu
  de l'environnement, s'ouvrent sans erreur de CSP. « Exporter un diagnostic » ouvre le dialogue de Windows devant
  la fenêtre, sur « Téléchargements », avec le nom `mcp-proxy-diagnostic-AAAAMMJJ-HHMM.zip` ; « Annuler » ne montre
  rien ; un emplacement sans droit d'écriture (`C:\Windows`) donne un bandeau rouge « Export de diagnostic
  impossible : accès refusé. » et aucun fichier.

## 019 — Installation au premier lancement

Préparation : un compte Windows **standard** (sans droits d'administrateur) dont le nom contient un accent (par
exemple « José »), sans installation précédente : ni `%LOCALAPPDATA%\Programs\MCP Proxy\`, ni raccourci
« MCP Proxy » dans le menu Démarrer, ni valeur `MCP Proxy` sous
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (`regedit`). Construire deux exe de versions différentes :
la version du dépôt, puis la même après avoir monté `version` dans `Cargo.toml` (par exemple `0.1.1`), copiés dans
`Téléchargements` sous `mcp-proxy-0.1.0.exe` et `mcp-proxy-0.1.1.exe`. Le journal de l'appli installée est dans
`%LOCALAPPDATA%\MCP Proxy\logs\`.

- [ ] **EX-019-04** — Double clic sur `mcp-proxy-0.1.0.exe` : la fenêtre « Installer MCP Proxy » s'affiche (texte
  en clair et en sombre selon le réglage de Windows, rien de coupé) ; « Installer » : la fenêtre se ferme, l'icône
  apparaît, la fenêtre principale propose le choix du profil. Le Gestionnaire des tâches (colonne « Ligne de
  commande ») montre `…\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe`, et plus aucun processus lancé depuis
  `Téléchargements`. Le menu Démarrer contient « MCP Proxy » ; il ouvre l'appli (ou ramène sa fenêtre). Le dossier
  d'installation ne contient que `mcp-proxy.exe` ; `%LOCALAPPDATA%\MCP Proxy\` (données) est un autre dossier.
  Noter si les notifications portent bien le nom et l'icône de MCP Proxy (raccourci sans AUMID, I019.10).
- [ ] **EX-019-05** — Quitter l'appli, supprimer le dossier d'installation et le raccourci, relancer
  `mcp-proxy-0.1.0.exe`, cocher « Démarrer MCP Proxy avec Windows », « Installer » : la valeur `Run` vaut `"C:\Users\José\AppData\Local\Programs\MCP
  Proxy\mcp-proxy.exe" --autostart`. Fermer la session, la rouvrir : l'icône apparaît, **aucune fenêtre** ne
  s'ouvre ; dans le menu de l'icône, « Démarrer avec Windows » est coché.
- [ ] **EX-019-06** — L'appli installée en cours, relancer `mcp-proxy-0.1.0.exe` : aucune fenêtre d'installation, la
  fenêtre principale de l'appli installée passe au premier plan ; un seul processus `mcp-proxy.exe` (hors ponts).
  Quitter l'appli et recommencer : l'appli installée démarre, rien n'est recopié (date de modification de l'exe
  installé inchangée).
- [ ] **EX-019-07** — Pendant toutes les étapes de cette section : aucune demande d'élévation (UAC), aucun écrit
  hors de `HKCU` et du profil de l'utilisateur.
- [ ] **EX-019-08** — Au premier double clic sur un exe téléchargé par un navigateur (marque « Mark of the Web ») :
  noter ce qu'affiche SmartScreen (« Windows a protégé votre ordinateur », bouton « Exécuter quand même »), puis si
  l'exe installé, relancé depuis le menu Démarrer, le montre encore. À reporter dans `028-authenticode.md`.
- [ ] **EX-019-15** — Appli 0.1.0 installée et en cours, un client LLM connecté (pont `stdio` sur l'exe installé) avec
  une session active. Lancer `mcp-proxy-0.1.1.exe` : la fenêtre « Mettre à jour MCP Proxy » dit « La version 0.1.0
  est installée. Cet exe est la version 0.1.1. ». « Lancer la version 0.1.0 » : rien n'est remplacé, la fenêtre de
  l'appli en cours passe devant. Relancer `mcp-proxy-0.1.1.exe`, « Mettre à jour » : l'icône disparaît puis revient ;
  le journal de l'ancienne instance se termine par « arrêt demandé par l'installation d'une nouvelle version de MCP
  Proxy » ; l'en-tête de la fenêtre affiche la version 0.1.1 ; la valeur `Run` est inchangée ; `mcp-proxy.exe.old`
  n'existe plus, ou existe tant que le pont de l'ancienne version tourne (il disparaît à la mise à jour suivante). Le
  client LLM retrouve ses outils après relance de son serveur MCP (la session est à rouvrir). Noter si un client qui
  relance aussitôt son pont fait repartir l'ancienne version (I019.7).
- [ ] **EX-019-16** — Échecs, rien ne doit rester (vérifier dossier, raccourci, valeur `Run`, fichiers `.new`) :
  (1) sans installation, créer un **fichier** nommé `MCP Proxy` dans `%LOCALAPPDATA%\Programs\`, lancer, cocher la
  case, « Installer » : message « Création du dossier d'installation impossible (…) », aucun reste ; supprimer le
  fichier. (2) Installation présente, rendre `%LOCALAPPDATA%\Programs\MCP Proxy\` en lecture seule pour
  l'utilisateur (Propriétés, Sécurité, refuser « Écriture »), lancer 0.1.1, « Mettre à jour » : message d'échec, l'exe
  installé est toujours la 0.1.0 et se lance. (3) Dans chaque cas, « Continuer sans installer » : l'appli tourne
  depuis `Téléchargements` (Gestionnaire des tâches), son menu a « Démarrer avec Windows » grisé ; « Quitter » ne
  laisse aucun processus. Fermer la fenêtre pendant « Installation en cours… » est sans effet.
- [ ] **EX-019-17** — Appli installée : dans le menu de l'icône, décocher « Démarrer avec Windows » : la valeur `Run`
  disparaît (`regedit`, F5) ; cocher : elle revient, identique ; quitter et relancer : l'état du menu suit la valeur.
  Modifier la valeur à la main (autre chemin) : le menu la montre décochée ; cocher la réécrit. Noter ce que montre le
  menu après « Désactiver » dans l'onglet « Applications de démarrage » du Gestionnaire des tâches (I019.9).
- [ ] **EX-019-18** — Sans installation, configurer un client LLM sur `…\Téléchargements\mcp-proxy-0.1.0.exe stdio`
  et le démarrer : aucune fenêtre d'installation ; l'appli démarre depuis `Téléchargements` (sa ligne de commande se
  termine par `--no-install`) et le client liste les outils.

## 025 — Module MCP local d'exemple (`jira-local`)

À dérouler quand l'appli (spec 011) et les fenêtres de saisie Basic (spec 012) existent, avec une config dont
l'instance `jira` pointe vers un Jira réel et dont `sso-tools` a une `probe_url` sur ce Jira.

- [ ] **EX-025-08** — Depuis un client LLM, rechercher des tickets (`rec_jira_search_issues`) : la saisie Basic de
  « SSO Outils » s'ouvre, puis la réponse liste les tickets attendus, sans adresse e-mail ni lien `self`. Récupérer
  un ticket (`prod_jira_get_issue`). Ajouter un commentaire en PROD (`prod_jira_add_comment`) : la confirmation
  s'affiche ; un refus n'ajoute rien dans Jira, une acceptation ajoute le commentaire. Changer le mot de passe dans
  Jira puis relancer une recherche : une nouvelle saisie est demandée, une seule, et la recherche aboutit.

