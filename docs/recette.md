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
- [ ] **EX-012-18** — Seulement si le SSO de recette repose sur Entra ID ou un compte Microsoft, sur un poste dont
  la session Windows porte ce compte. D'abord la sonde avec le même argument,
  `cargo run -p mcp-proxy --example webview_probe -- --sso-windows` : son bilan doit rester « 8/8 RÉUSSI »
  (l'argument ne doit casser ni le mode InPrivate ni le partage du profil). Puis compiler l'appli avec
  `os_primary_account_sso = true` dans `[webview]` de `config/default.toml`, sans la commiter : un appel d'outil
  ouvre la fenêtre de login, et la page Microsoft reconnaît le compte de la session Windows sans saisie de
  l'identifiant. Recompiler avec `false` : la même page demande l'identifiant. L'effet de cet argument en mode
  InPrivate n'est pas attesté par les sources : si le compte n'est pas reconnu avec `true`, le noter ici ; le
  réglage est alors sans effet, et il faudra décider de le retirer ou de le remplacer.

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
  le journal de l'ancienne instance se termine par « arrêt demandé par l'installation ou la désinstallation de
  MCP Proxy » ; l'en-tête de la fenêtre affiche la version 0.1.1 ; la valeur `Run` est inchangée ; `mcp-proxy.exe.old`
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

## 020 — Updater Artifactory : signature ed25519, remplacement de l'exe, canal bêta

Préparation (une fois) :

1. Un dépôt **générique** sur un Artifactory de test, lisible sans identifiant (par exemple `generic-test-local`), et
   un compte qui peut y déposer des fichiers (interface web ou `jf rt upload`).
2. Une clé de signature de recette, **hors du dépôt** : `cargo xtask keygen --out C:\cles\recette.key`. La commande
   affiche la clé publique (64 caractères hexadécimaux) et son `key-id` ; la clé privée n'est que dans le fichier.
   Copier la clé publique sur la ligne non commentée de `crates/deploy/keys/current.pub` (modification locale, **à ne
   pas committer**). Contrôle : `cargo xtask check-release` affiche « clé courante : … » et « OK ».
3. Dans `config/default.toml`, section `[update]` (modification locale, à ne pas committer) : `base_url` = l'URL de
   l'Artifactory de test (`https://…/artifactory`), `repository` = le dépôt, `path = "mcp-proxy"`,
   `check_interval_hours = 1`.
4. Construire et installer la version du dépôt (0.1.0) comme en `EX-019-04`. Le journal est dans
   `%LOCALAPPDATA%\MCP Proxy\logs\` et dans l'onglet Journal.
5. Pour chaque version à publier : monter `version` dans le `Cargo.toml` racine, construire, puis signer en
   PowerShell : `$env:ED25519_PRIVATE_KEY = Get-Content C:\cles\recette.key` puis
   `cargo xtask sign --exe target\release\mcp-proxy.exe --version X.Y.Z` (ajouter `--critical true` pour une release
   critique), et vérifier : `cargo xtask verify --exe target\release\mcp-proxy.exe`. Ni l'une ni l'autre commande
   n'affiche la clé privée. Déposer `mcp-proxy.exe` et `mcp-proxy.exe.sig` dans `mcp-proxy/X.Y.Z/` du dépôt.
   Remettre `ED25519_PRIVATE_KEY` à vide après la recette.

La première vérification a lieu une minute après le démarrage de l'appli, puis toutes les heures ; cocher ou décocher
« Recevoir les préversions » dans le menu de l'icône en lance une tout de suite. Chaque publication ci-dessous porte un
numéro plus élevé que la précédente : l'updater n'essaie que la plus haute.

- [ ] **EX-020-08** — Publier `0.1.1` signée. Dans la minute qui suit le démarrage de l'appli 0.1.0 installée : le
  journal dit « mise à jour 0.1.1 prête : elle sera utilisée au prochain démarrage de MCP Proxy », une notification
  « Mise à jour prête » s'affiche, l'en-tête montre « Redémarrer pour mettre à jour ». Dans
  `%LOCALAPPDATA%\Programs\MCP Proxy\` : `mcp-proxy.exe` est le nouveau fichier (date, taille) et
  `mcp-proxy.exe.old` l'ancien ; `%LOCALAPPDATA%\MCP Proxy\update\` est vide. Le bouton ouvre un dialogue qui
  prévient que les connexions seront fermées ; « Annuler » ne change rien ; « Redémarrer » : l'icône disparaît puis
  revient, l'en-tête affiche « version 0.1.1 », les sessions sont à rouvrir, `mcp-proxy.exe.old` a disparu. Le
  Gestionnaire des tâches ne montre qu'un processus de l'appli.
- [ ] **EX-020-09** — Publier `0.1.2` avec un `.sig` modifié à la main (un caractère de la ligne `sha256:` changé) :
  le journal dit « vérification des mises à jour en échec : signature du manifeste invalide », sans adresse ni
  chemin ; aucune notification, aucun bouton, l'exe installé n'a pas changé (date), `update\` est vide. Publier
  `0.1.3` dont le `.sig` est celui de l'exe construit mais dont l'exe déposé est un autre fichier (par exemple l'exe
  0.1.0) : « empreinte SHA-256 de l'exe différente de celle du manifeste », même absence d'effet.
- [ ] **EX-020-10** — Déposer l'exe et le `.sig` de `0.1.1`, tels quels, sous `mcp-proxy/0.1.4/` : le journal dit
  « le manifeste du dossier 0.1.4 annonce la version 0.1.1 », rien n'est remplacé. Signer l'exe 0.1.1 en prétendant
  qu'il est 0.1.5 (`cargo xtask sign --exe … --version 0.1.5`) : la commande refuse (« l'exe porte la version
  0.1.1 »).
- [ ] **EX-020-11** — Publier `0.2.0-rc.1` signée : par défaut, rien (le fichier de log dit « aucune version plus
  récente », le journal ne montre rien) ; cocher « Recevoir les préversions » dans le menu de l'icône : une
  vérification part aussitôt, et `0.2.0-rc.1` est mise en place comme en `EX-020-08`. Quitter et relancer : la case
  est restée cochée. Lancée hors de l'installation (`--no-install`), l'appli montre la case grisée et ne contacte pas
  Artifactory.
- [ ] **EX-020-19** — Publier `0.2.1` signée avec `--critical true` (décocher d'abord les préversions si besoin).
  Avant la vérification, laisser une confirmation en attente (appel d'un outil `destructive` depuis un client LLM,
  sans y répondre). La notification « Mise à jour critique » annonce le redémarrage et la reconnexion ; l'en-tête
  affiche « Mise à jour critique » à côté du bouton. L'appli ne redémarre pas tant que la confirmation attend (même
  après plus d'une minute) ; une fois la confirmation refusée et plus aucun appel en cours, elle redémarre seule :
  le journal de l'ancienne instance se termine par « redémarrage automatique pour la mise à jour critique 0.2.1 ».
- [ ] **EX-020-20** — Pendant `EX-020-08` (ou une publication suivante), garder un client LLM ouvert, connecté par le
  pont `stdio` à l'exe installé : la mise en place réussit malgré l'exe tenu, le client continue de lister et
  d'appeler les outils jusqu'au redémarrage, puis retrouve ses outils après relance de son serveur MCP. Tant que le
  pont de l'ancienne version tourne, `mcp-proxy.exe.old` peut rester ; quitter le client puis relancer l'appli : il
  a disparu. Noter si un `.old` encore tenu fait échouer la mise à jour suivante (« ancienne version encore utilisée
  par un client LLM »).

## 021 — Pipeline GitLab de release

Recette **GitLab**, pas Windows : le pipeline n'a jamais tourné avant elle (aucun GitLab ni runner dans
l'environnement de développement ; seul `cargo xtask check-ci` y est exécuté). À dérouler au premier tag, dans
l'ordre ci-dessous, en gardant les journaux des jobs.

Préparation :

1. Dérouler « Préparer une fois » de `docs/release.md` : clé de release dans `crates/deploy/keys/current.pub`
   (fusionnée), tags `v*` protégés, variables `ED25519_PRIVATE_KEY`, `JF_ACCESS_TOKEN`, `JF_URL`,
   `ARTIFACTORY_REPO` (protégées, portée `release`), runner et `WINDOWS_BUILD_MODE`, dépôt Artifactory, section
   `[update]` de `config/default.toml`. Pour une première fois, un projet GitLab de test (copie du dépôt), un dépôt
   Artifactory de test et une clé de recette évitent de consommer de vrais numéros de version.
2. En local : `cargo xtask check-ci` affiche « check-ci : OK ».
3. Points jamais vérifiés, à regarder au passage et à noter s'ils diffèrent : variable dans `tags`
   (`$WINDOWS_RUNNER_TAG`) ; arrêt d'un job PowerShell par `if (-not $?) { exit 1 }` ; variables de portée
   `release` reçues par `sign` (`action: prepare`) et `publish` ; `-o ci.variable` sur une poussée de tag ; image
   JFrog CLI avec `entrypoint: [""]`, `curl` et `cmp` ; `JF_URL` et `JF_ACCESS_TOKEN` lus par `jf` ; en mode
   `linux-xwin`, image, SDK, `llvm-rc` et Node 22 dans l'image ; bit d'exécution de `dist/update-tool` conservé
  dans l'artefact (le job le remet de toute façon).

- [ ] **EX-021-03** — Ouvrir une demande de fusion (changement anodin) : son pipeline est de type « merge request »
  et ne contient que `lint` et `test`, aucun `test:windows`, `build:…`, `sign` ni `publish`. Le journal de `test`
  contient « check-ci : OK ». Pousser un nouveau commit sur la branche : un seul pipeline (celui de la demande de
  fusion), pas de pipeline de branche en double. Une branche sans demande de fusion : pipeline de branche, `lint` et
  `test` seulement.
- [ ] **EX-021-12** — Sur un commit dont la version est `X.Y.Z`, créer le tag protégé `vX.Y.(Z+99)` : `lint` et
  `test` passent ; `build:windows` (ou `build:linux-xwin`) échoue sur « check-tag : ÉCHEC : le tag … ne correspond
  pas à la version du workspace », sans aucune ligne `npm` ni `cargo build` après ; `sign` et `publish` ne tournent
  pas ; rien n'apparaît dans Artifactory. En mode `windows-runner`, c'est aussi la preuve qu'une commande en échec
  arrête un job PowerShell. Supprimer ce tag. Un tag non protégé (hors `v*`) : `lint` et `test` seulement.
- [ ] **EX-021-04** — Créer et pousser le tag protégé `vX.Y.Z` de la version du workspace. Le pipeline montre `lint`,
  `test`, `test:windows` (mode `windows-runner`), `build:…`, `build:release-tools`, `sign`, `publish` ; aucun job `authenticode`. Journaux :
  `check-tag : vX.Y.Z = version X.Y.Z du workspace (canal stable)` ; dans `sign`, « check-release : OK » avec la
  clé courante attendue, puis `dist/mcp-proxy.exe.sig : version X.Y.Z, sha256 …, critical false, key-id …` et
  « vérifié : … » avec ce même `key-id` ; `publish` se termine par « publié dans … ». Dans Artifactory,
  `mcp-proxy/X.Y.Z/` contient exactement `mcp-proxy.exe` et `mcp-proxy.exe.sig`, lisibles sans identifiant
  (fenêtre de navigation privée). Sur un poste Windows, télécharger les deux fichiers :
  `cargo xtask verify --exe mcp-proxy.exe --sig mcp-proxy.exe.sig` répond « vérifié » ; `Get-FileHash
  mcp-proxy.exe` donne le `sha256` du `.sig`. L'exe se lance et s'installe (section 019). En mode `linux-xwin`,
  vérifier en plus qu'il démarre sur un poste sans « Microsoft Visual C++ Redistributable » (bibliothèque
  d'exécution liée statiquement).
- [ ] **EX-021-05** — Relancer le job `publish` du même pipeline (« Retry ») : il échoue sur « mcp-proxy/X.Y.Z existe
  déjà ou Artifactory ne répond pas (HTTP 200), rien n'est envoyé », sans ligne `jf rt upload` exécutée ; dans
  Artifactory, les deux fichiers gardent leur date et leur empreinte. Lancer un nouveau pipeline pour le même tag
  (*Build > Pipelines > Run pipeline*) : même échec de `publish`. Contrôle de la vraie garantie : avec le jeton du
  pipeline, `jf rt upload` d'un fichier quelconque sur `mcp-proxy/X.Y.Z/mcp-proxy.exe` est refusé (droit
  d'écrasement absent).
- [ ] **EX-021-06** — Installer l'exe `X.Y.Z` publié par le pipeline (section 019). Monter la version à `X.Y.(Z+1)`,
  fusionner, pousser le tag : une fois publiée, l'appli installée en `X.Y.Z` la trouve dans la minute qui suit son
  démarrage (« mise à jour X.Y.(Z+1) prête »), et le redémarrage lance `X.Y.(Z+1)` (comme `EX-020-08`).
- [ ] **EX-021-15** — Dans le pipeline de `EX-021-04`, le journal de `sign` ne contient aucune ligne `Compiling`,
  `Downloading`, `rustup` ni `npm` : seulement `chmod +x dist/update-tool` et les trois commandes
  `dist/update-tool …`, avec leurs résultats. `build:release-tools` (sans environnement dans la page *Operate >
  Environments*) a produit l'artefact `dist/update-tool`. En mode `linux-xwin`, avec une image sans Node (ou avec un
  autre Node que le 22) : `build:linux-xwin` s'arrête sur « XWIN_IMAGE doit contenir Node 22 … » avant toute ligne
  `rustup` ou `cargo`, et rien n'est publié.
- [ ] **EX-021-10** — Chercher dans les journaux de tous les jobs du pipeline les huit premiers caractères de la clé
  privée et du jeton : absents. Dans *Settings > CI/CD > Variables*, les quatre variables sont protégées, la clé et
  le jeton masqués, portée `release`. Dans un projet de test seulement, ajouter temporairement à `test` la ligne
  `test -n "$ED25519_PRIVATE_KEY" && echo "clé présente" || echo "clé absente"` et pousser un tag protégé : « clé
  absente » (la portée `release` la réserve à `sign` et `publish`). Ne pas fusionner cette ligne.
- [ ] **EX-021-11** — Publier `X.Y.(Z+2)` avec `git push origin vX.Y.(Z+2) -o ci.variable="RELEASE_CRITICAL=true"` :
  `sign` affiche `critical true`, le `.sig` téléchargé contient `critical: true`, et l'appli installée redémarre
  d'elle-même comme en `EX-020-19`. Puis monter la version à `X.(Y+1).0-rc.1` et pousser `vX.(Y+1).0-rc.1` :
  `check-tag` affiche « (préversion, canal bêta) », la version est publiée sous `mcp-proxy/X.(Y+1).0-rc.1/`,
  ignorée par un poste qui ne reçoit pas les préversions et proposée dès que la case est cochée (comme `EX-020-11`).

## 022 — Désinstallation propre

**À faire d'abord sur un poste ou un compte Windows de test** : la désinstallation supprime des fichiers et des
clés du registre. Préparation : un compte Windows **standard** (sans droits d'administrateur), MCP Proxy installé
(spec 019), lancé une fois, un profil choisi, une connexion ouverte (pour que `logs\`, `cache\`, `webview\`,
`settings.json`, `settings.key` existent), « Démarrer avec Windows » coché, une notification reçue. Claude Desktop et
VS Code configurés par « Clients LLM… » (spec 023). Avant de commencer, noter avec `regedit` et l'Explorateur : la
valeur `MCP Proxy` de `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, la clé
`HKCU\Software\Classes\AppUserModelId\McpProxy.Desktop`, la valeur `mcp-proxy.exe` de
`HKCU\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications`, la clé
`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy`, les dossiers
`%LOCALAPPDATA%\Programs\MCP Proxy\`, `%LOCALAPPDATA%\MCP Proxy\` et `%LOCALAPPDATA%\com.mcpproxy.app\`, le
raccourci `%APPDATA%\Microsoft\Windows\Start Menu\Programs\MCP Proxy.lnk`, et copier à part les fichiers de
configuration des clients LLM.

- [ ] **EX-022-03** — Paramètres de Windows, « Applications », « Applications installées » : « MCP Proxy » est
  listé avec la version affichée dans l'en-tête de l'appli, l'éditeur « MCP Proxy », une taille proche de celle de
  l'exe et l'icône de l'appli (noter si Windows l'affiche) ; le menu « … » ne propose que « Désinstaller » (ni
  « Modifier » ni « Réparer »). Dans `regedit`, la clé `…\Uninstall\MCP Proxy` a `UninstallString` égal à
  `"C:\Users\…\AppData\Local\Programs\MCP Proxy\mcp-proxy.exe" --uninstall`. Après une mise à jour par la
  fenêtre d'installation (`EX-019-15`), la version listée est la nouvelle.
- [ ] **EX-022-04** — Fermer Claude Desktop et VS Code. « Désinstaller » dans « Applications installées » : la
  fenêtre « Désinstaller MCP Proxy » s'ouvre (en clair et en sombre selon Windows, rien de coupé), case « Retirer MCP
  Proxy des clients LLM » cochée, qui nomme les clients configurés. « Désinstaller » : l'icône de notification
  disparaît, le compte rendu « MCP Proxy est désinstallé » s'affiche ; « Fermer ». Vérifier : plus de raccourci dans
  le menu Démarrer, plus de valeur `Run`, plus de clé AUMID, plus de valeur WER `mcp-proxy.exe`, plus d'entrée
  `Uninstall\MCP Proxy` ni de ligne dans « Applications installées » (actualiser), plus de dossiers
  `Programs\MCP Proxy`, `MCP Proxy` et `com.mcpproxy.app` sous `%LOCALAPPDATA%` (attendre quelques secondes après la
  fermeture). `%LOCALAPPDATA%\Programs\` lui-même est toujours là. Dans chaque fichier de client : seule l'entrée
  `mcp-proxy` a disparu, une sauvegarde `….bak-<date>` est à côté. Fermer puis rouvrir la session Windows : MCP
  Proxy ne démarre pas. Regarder `%TEMP%` : aucun fichier `.mcp-proxy.*.__relocated__.exe` ni
  `.mcp-proxy.*.__selfdelete__.exe` ne reste (noter sinon).
- [ ] **EX-022-05** — Pendant toute cette section : aucune demande d'élévation (UAC), aucune écriture hors de `HKCU`
  et du profil de l'utilisateur.
- [ ] **EX-022-13** — Réinstaller, configurer Claude Desktop, le démarrer et appeler un outil (un pont tourne).
  Lancer la désinstallation **sans** fermer Claude Desktop. Après « Désinstaller » : l'appli s'arrête, Claude Desktop
  signale que son serveur MCP s'est arrêté et ne relance pas l'appli (Gestionnaire des tâches : aucun nouveau
  `mcp-proxy.exe` hors du pont). Noter ce que dit le compte rendu, ce qui reste dans
  `%LOCALAPPDATA%\Programs\MCP Proxy\` et dans `%TEMP%` après la fermeture de la fenêtre, puis après avoir quitté
  Claude Desktop (le pont tient l'exe : un `.mcp-proxy.*.__relocated__.exe` peut rester dans `%TEMP%`, I022.4).
- [ ] **EX-022-14** — Réinstaller et lancer MCP Proxy une fois. Ouvrir le journal du jour
  (`%LOCALAPPDATA%\MCP Proxy\logs\mcp-proxy-<date>.log`) dans un programme qui le garde ouvert en écriture
  (par exemple `powershell -Command "$f=[IO.File]::Open('<chemin>','Open','ReadWrite','None'); Read-Host"`). Désinstaller : le
  compte rendu « Désinstallation incomplète » nomme le fichier en rouge, dit que MCP Proxy reste installé ;
  « Applications installées » liste toujours MCP Proxy, l'exe est à sa place (`mcp-proxy.exe`, pas `.old`). Fermer le
  programme qui tenait le fichier, relancer « Désinstaller » : tout part.
- [ ] **EX-022-15** — Avec MCP Proxy installé, lancer `mcp-proxy.exe --uninstall` depuis une copie de l'exe dans
  `Téléchargements` : message « Cet exe n'est pas celui qui est installé », rien n'est modifié. Sans installation
  (après `EX-022-04`), même commande : message « MCP Proxy n'est pas installé sur ce compte », rien n'est créé.
  Avec l'appli installée ouverte et bloquée (en recette : ne pas pouvoir la fermer est difficile à provoquer ; à
  défaut, noter que le cas est couvert par les tests) : « La désinstallation n'a pas eu lieu », « Réessayer ».
- [ ] **EX-022-16** — Installer une version construite **avant** cette spec (commit précédant `Spec: 022`), puis la
  mettre à jour vers celle-ci par la fenêtre d'installation ou par l'updater (spec 020) : après le premier démarrage
  de la nouvelle version, « Applications installées » liste MCP Proxy avec la nouvelle version.

## 023 — Configuration automatique des clients LLM

Préparation : MCP Proxy installé (spec 019), un profil choisi avec au moins un environnement exposé et une session
active. Claude Desktop et VS Code installés, chacun démarré une fois. Avant chaque étape qui écrit, copier à part le
fichier visé (le dialogue le sauvegarde aussi, mais la recette ne doit pas en dépendre). Ouvrir le dialogue par le
bouton « Menu » de l'en-tête, « Clients LLM… ». Dans Claude Desktop, la liste des serveurs se voit dans Settings,
Developer ; dans VS Code, par la commande « MCP: List Servers ».

- [ ] **EX-023-06** — Claude Desktop (ligne qui correspond à l'installation, voir `EX-023-17`), avec dans son fichier
  un autre serveur qui a un `env` : « Installer », lire l'aperçu (fichier, lignes `+`, « L'autre serveur et le reste
  du fichier ne sont pas modifiés », sauvegarde ; aucune valeur de l'`env` affichée), « Installer ». Le message dit
  de quitter Claude Desktop depuis la zone de notification ; le faire (clic droit, « Quit »), le relancer : les
  outils de MCP Proxy apparaissent et un appel de lecture réussit. Le fichier contient `"command":
  "C:\\Users\\…\\AppData\\Local\\Programs\\MCP Proxy\\mcp-proxy.exe"` et `"args": ["stdio"]` ; la
  sauvegarde `claude_desktop_config.json.bak-<date>` est à côté, identique à la copie faite avant. Rouvrir le
  dialogue : « configuré », « Installer » inactif.
- [ ] **EX-023-07** — VS Code : même chose sur `%APPDATA%\Code\User\mcp.json` préparé avec un commentaire `//`, une
  virgule finale et un autre serveur (ou absent : il est alors créé). Après « Installer » : le fichier garde le
  commentaire, la virgule, l'autre serveur et ses fins de ligne (ouvrir dans le Bloc-notes : CRLF conservé si le
  fichier en avait) ; recharger la fenêtre de VS Code (« Developer: Reload Window ») : `mcp-proxy` est listé,
  démarre, et ses outils sont proposés à Copilot.
- [ ] **EX-023-08** — Pour chaque client configuré : « Retirer », lire l'aperçu (ligne `-` de l'entrée, « Seule
  l'entrée « mcp-proxy » est retirée »), « Retirer » ; comparer avec la copie faite avant l'installation : seule
  l'entrée `mcp-proxy` a disparu, les autres serveurs, les autres clés et les commentaires sont là ; le client
  relancé ne liste plus MCP Proxy et ses autres serveurs marchent.
- [ ] **EX-023-17** — Claude Desktop installé par son installeur officiel (paquet MSIX) : vérifier que le dossier
  `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\` existe (sinon noter le vrai nom du dossier du paquet : le nom
  écrit dans l'appli est à corriger). Dans le dialogue, les deux lignes « installation classique » et « paquet MSIX »,
  la note des deux fichiers, la ligne classique « sans doute pas lu ». Installer **seulement** sur la ligne
  classique, redémarrer Claude Desktop : noter si MCP Proxy apparaît (attendu : non). Retirer, installer seulement
  sur la ligne du paquet, redémarrer : noter si MCP Proxy apparaît (attendu : oui). Ouvrir « Edit Config » de Claude
  Desktop : noter quel fichier il ouvre. Redémarrer Windows, puis mettre à jour Claude Desktop si une version est
  proposée : noter si le fichier du paquet a été réécrit ou vidé. Sur une installation classique (installeur
  `.exe`), vérifier qu'une seule ligne est installée. Reporter les constats dans la section « Adaptateurs » de la
  spec 023 et dans `DECISIONS.md` (I023.3, I023.4).
- [ ] **EX-023-18** — Dans l'appli, en clair puis en sombre : le menu de l'en-tête s'ouvre et le dialogue s'affiche
  sans erreur de CSP ni texte coupé. « Copier la configuration », puis coller dans le Bloc-notes : le JSON
  `mcpServers` avec le chemin de l'exe installé (si la WebView refuse la copie, le dialogue affiche « Copie
  impossible » et le texte à sélectionner : le noter, I023.12). Claude Desktop **en cours** : ouvrir l'aperçu
  « Installer », modifier puis enregistrer `claude_desktop_config.json` dans le Bloc-notes, cliquer « Installer » :
  « Le fichier a changé depuis l'aperçu », nouvel aperçu, rien d'écrit (le fichier est celui du Bloc-notes) ; le
  journal dit « changée depuis l'aperçu ». Rendre le fichier invalide (supprimer une virgule) : ligne « fichier
  illisible » en rouge avec le numéro de ligne, « Installer » donne l'extrait à coller, rien n'est écrit. Quitter
  MCP Proxy, renommer le dossier `%LOCALAPPDATA%\Programs\MCP Proxy`, lancer l'exe téléchargé avec `--no-install` :
  le dialogue dit que MCP Proxy n'est pas installé, « Installer » est inactif, « Retirer » marche ; remettre le
  dossier. Le journal ne contient aucun chemin ni contenu de fichier.

## 024 — Auto-diagnostic (`doctor`)

Préparation : MCP Proxy installé (spec 019), lancé, un profil choisi avec au moins un environnement exposé, un client
LLM configuré (spec 023). Noter l'heure, et le contenu de `%LOCALAPPDATA%\MCP Proxy\logs\` avant de commencer.
Hypothèses à confirmer au passage (I024.9) : `cmd` n'attend pas la fin d'un exe fenêtré tapé à l'invite, d'où le
fichier `.cmd` ou `start /wait` ci-dessous ; un exe fenêtré lancé sans redirection a une sortie standard nulle (pas de
texte dans la console, une boîte de message).

- [ ] **EX-024-09** — Créer `test-doctor.cmd` contenant les deux lignes `"%LOCALAPPDATA%\Programs\MCP
  Proxy\mcp-proxy.exe" doctor > "%USERPROFILE%\Desktop\rapport.txt"` et `echo code %ERRORLEVEL%`, le lancer depuis
  un terminal (`cmd`) : aucune fenêtre de MCP Proxy ne s'ouvre et l'icône de l'instance en cours ne bouge pas ; le
  terminal affiche `code 0`, `1` ou `2` ; `rapport.txt` commence par « MCP Proxy — auto-diagnostic », sa ligne
  « Résultat » correspond au code (0 : tout va bien, 1 : avertissements, 2 : au moins un échec), et sa dernière
  ligne donne `%LOCALAPPDATA%\MCP Proxy\logs\doctor-AAAAMMJJ-HHMMSS.txt` : ce fichier existe et a le même contenu.
  Aucune boîte de message (la sortie est redirigée). Variante PowerShell : `$p = Start-Process -FilePath
  "$env:LOCALAPPDATA\Programs\MCP Proxy\mcp-proxy.exe" -ArgumentList doctor -RedirectStandardOutput rapport.txt
  -Wait -PassThru; $p.ExitCode`. Puis, dans le terminal, taper `mcp-proxy.exe doctor` **sans** redirection (depuis le
  dossier d'installation) : rien ne s'écrit dans la console, une boîte « MCP Proxy — auto-diagnostic » donne le
  résultat et le chemin du nouveau fichier ; coller ce chemin dans la barre d'adresse de l'Explorateur : le rapport
  s'ouvre. Relancer deux fois de suite dans la même seconde (`.cmd` avec deux lignes) : deux fichiers, le second
  suffixé `_2`. Vérifier dans chaque rapport : la ligne « Appli » dit « instance en cours en version … », aucun
  chemin ne contient le nom de l'utilisateur, aucune URL complète, aucune valeur des fichiers des clients LLM. Le
  journal de l'appli (onglet Journal, et `mcp-proxy-AAAA-MM-JJ.log`) ne contient aucune ligne due au diagnostic.
  Quitter MCP Proxy, relancer le `.cmd` : « Appli » en avertissement « MCP Proxy n'est pas lancé », code 1 au moins ;
  aucune instance n'a été lancée par le diagnostic (Gestionnaire des tâches).
- [ ] **EX-024-06** — Paramètres de Windows, « Système », « Notifications » : désactiver les notifications de
  « MCP Proxy ». Relancer le `.cmd` : « Notifications » en avertissement, « notifications désactivées pour MCP
  Proxy », avec la marche à suivre (« Paramètres de Windows, « Système », « Notifications »… »). Les réactiver : le
  contrôle repasse « ok ». Activer « Ne pas déranger » : avertissement « mode « Ne pas déranger » actif ». Le noter si
  Windows ne présente pas l'appli dans la liste tant qu'aucune notification n'a été montrée.
- [ ] **EX-024-07** — Dans l'appli, menu de l'en-tête, « Auto-diagnostic », en clair puis en sombre : « Contrôle en
  cours… » quelques secondes au plus, puis les onze contrôles, chacun avec son issue (noir et blanc pour « OK »,
  orange pour un avertissement, rouge pour un échec) et la marche à suivre ; la ligne « Appli » dit « cette instance
  en version … », l'appli ne s'arrête pas et sa fenêtre ne se rouvre pas. Le rapport affiché est celui du fichier
  `doctor-….txt` indiqué en bas (comparer à l'écran). « Copier le rapport », coller dans le Bloc-notes : le texte
  du fichier (si la WebView refuse la copie, le dialogue affiche le texte à sélectionner : le noter). Aucune erreur
  de CSP (outils de développement fermés : rien à vérifier de plus), aucun texte coupé. Puis « Exporter un
  diagnostic » (onglet Journal) : l'archive contient `auto-diagnostic.txt`, rapport refait à l'export (« Rapport
  joint à l'export de diagnostic » à la fin), et aucun nouveau `doctor-*.txt` n'est apparu dans le dossier des logs.

Contrôles à provoquer une fois chacun, en notant ce que dit le rapport (attendu entre parenthèses) :

- Laisser un `mcp-proxy.exe.old` dans `%LOCALAPPDATA%\Programs\MCP Proxy\` (copier l'exe sous ce nom) :
  « Installation » en avertissement (reste d'une mise à jour) ; le supprimer ensuite. Supprimer l'entrée
  « Applications installées » (`reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\MCP Proxy"
  /f`) : avertissement « absente » ; quitter et relancer MCP Proxy depuis le menu Démarrer : l'entrée revient et le
  contrôle repasse « ok ». Mettre une autre commande dans la valeur `Run` « MCP Proxy » : avertissement.
- Couper le VPN (ou le réseau) : « MCP » en échec « nom introuvable (DNS) » ou « connexion impossible » pour chaque
  instance exposée, « Artifactory » en échec ; aucune instance d'un environnement non exposé n'est citée.
- Décaler l'horloge de Windows de 10 minutes (synchronisation automatique coupée), réseau rétabli : « Horloge » en
  échec « en avance de 10 min » ; la remettre à l'heure.
- Un exe construit hors du pipeline (le cas de cette recette) : « Clé de mise à jour » en avertissement « aucune clé
  de mise à jour embarquée ».
- Avec une capture réseau (Fiddler, ou le journal d'un proxy) pendant le diagnostic : une seule requête `GET /` par
  origine des instances exposées, sans en-tête `Authorization` ni `Cookie`, même avec des sessions connectées ; et
  la requête de l'API Storage d'Artifactory. Aucune autre destination.

## 025 — Module MCP local d'exemple (`jira-local`)

À dérouler quand l'appli (spec 011) et les fenêtres de saisie Basic (spec 012) existent, avec une config dont
l'instance `jira` pointe vers un Jira réel et dont `sso-tools` a une `probe_url` sur ce Jira.

- [ ] **EX-025-08** — Depuis un client LLM, rechercher des tickets (`rec_jira_search_issues`) : la saisie Basic de
  « SSO Outils » s'ouvre, puis la réponse liste les tickets attendus, sans adresse e-mail ni lien `self`. Récupérer
  un ticket (`prod_jira_get_issue`). Ajouter un commentaire en PROD (`prod_jira_add_comment`) : la confirmation
  s'affiche ; un refus n'ajoute rien dans Jira, une acceptation ajoute le commentaire. Changer le mot de passe dans
  Jira puis relancer une recherche : une nouvelle saisie est demandée, une seule, et la recherche aboutit.

