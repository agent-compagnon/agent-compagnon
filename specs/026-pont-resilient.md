---
id: "026"
titre: "Pont stdio résilient : survivre au redémarrage de l'appli"
etape: "9"
statut: "en cours"
depend_de: ["011", "020"]
suivi: "github#27"
---

# 026 — Pont stdio résilient : survivre au redémarrage de l'appli

> Proposé pendant le cadrage, hors QCM ; retenu par l'utilisateur le 2026-11-23 (D19).

## Pourquoi

Chaque client LLM lance `mcp-proxy.exe stdio`, qui relaie vers l'appli. Quand l'appli redémarre (mise à jour,
plantage, « Quitter » puis relance), le pipe se ferme, le pont s'arrête, et le client considère son serveur MCP
comme mort. Claude Desktop ne le relance pas : il faut redémarrer Claude Desktop. Avec des mises à jour
automatiques, cela arriverait régulièrement.

## À faire

- Quand le pipe se ferme, le pont **ne s'arrête pas**. Il attend le retour de l'appli (nouvelles tentatives pendant
  60 s, relance de l'appli si elle n'est pas là **et qu'elle ne s'est pas arrêtée volontairement**), puis se
  reconnecte.
- **Chaque reconnexion refait la preuve mutuelle par DPAPI** de la spec 011 avant tout octet MCP, rejeu de
  `initialize` compris : un pipe usurpé pendant que l'appli était arrêtée ne doit rien recevoir. Preuve en échec :
  nouvelle tentative dans le délai de 60 s, sans jamais relayer.
- **Ancien protocole** (handshake) : le pont garde en mémoire la requête `initialize` et la notification
  `initialized` du client. À la reconnexion, il les rejoue vers la nouvelle instance et absorbe la réponse, que le
  client a déjà reçue.
- **2026-07-28** (sans session) : rien à rejouer.
- Requêtes en cours au moment de la coupure : le pont répond lui-même une erreur claire pour chacune
  (`MCP Proxy redémarre, réessayez dans un instant.`). Il suit donc les identifiants des requêtes sans réponse.
- Après reconnexion, les secrets ont disparu : la connexion à la demande reprend normalement au premier appel.
- Le pont reste sans secret et sans logique métier. Il ne lit des messages que ce qu'il faut pour suivre les
  identifiants et repérer `initialize` et `initialized`.
- Au-delà de 60 s sans appli : le pont s'arrête avec un message sur stderr, comme aujourd'hui.

Réalisation (amendement du 2026-11-23) :

- **Messages** : une ligne, un message JSON-RPC (transport stdio de MCP). Le pont lit l'identifiant et la méthode de
  chaque message (et de chaque élément d'un lot), l'identifiant visé par `notifications/cancelled`, et, dans la seule
  réponse à `initialize`, `protocolVersion` et `capabilities` ; rien d'autre, ni paramètres ni résultats. Un message
  non JSON-RPC est relayé tel quel, sans suivi. Une ligne de plus de 64 Mio (quatre fois ce que rmcp accepte) est
  ignorée jusqu'à sa fin de ligne, comme rmcp le fait. Une ligne non terminée à la fin d'un flux n'est jamais relayée
  ni rejouée : un message à moitié reçu au moment de la coupure est perdu, et sa requête reçoit l'erreur.
- **Pendant la connexion** : chaque sens est relayé message par message, indépendamment de l'autre (un tampon plein
  dans un sens ne bloque pas l'autre). Une requête du client est notée avant d'être écrite vers le pipe, une réponse
  du serveur la retire avant d'être écrite vers le client ; une annulation (`notifications/cancelled`) retire la
  requête visée : aucune réponse ne lui est plus due. Les requêtes du serveur vers le client (elicitation, `ping`…)
  sont suivies de même. Entrée du client fermée : le pipe est fermé en écriture, les réponses encore dues sont
  relayées, puis le pont s'arrête (code 0).
- **À la coupure** : chaque requête du client sans réponse reçoit **une** erreur JSON-RPC (code `-32000`, message
  `MCP Proxy redémarre, réessayez dans un instant.`, son identifiant). Chaque requête du serveur qui attendait le
  client est annulée auprès de lui (`notifications/cancelled`) ; sa réponse tardive n'est relayée à personne ; si la
  nouvelle instance émet une requête sous le même identifiant avant cette réponse, le pont la refuse lui-même à la
  nouvelle instance (erreur), sans la montrer au client : une réponse tardive ne peut pas être prise pour la sienne.
- **Pendant la coupure** : chaque requête du client reçoit aussitôt la même erreur, sans attendre l'appli (une
  requête gardée pourrait s'exécuter après que le client a abandonné, et produire son effet deux fois) ; un
  `initialized` est gardé pour le rejeu ; les autres messages sont abandonnés. Entrée du client fermée : arrêt
  immédiat (code 0).
- **Reconnexion** : un essai toutes les 100 ms pendant 60 s à compter de la coupure. Chaque connexion fait la preuve
  en rôle `pont` (spec 011) ; en échec, rien n'est envoyé et l'essai suivant suit. Ancien protocole : `initialize`
  tel que le client l'a écrit, réponse attendue 5 s au plus (ce que la nouvelle instance envoie avant est abandonné),
  puis `initialized` si le client l'avait envoyé. La réponse doit porter la même `protocolVersion` et les mêmes
  `capabilities` que celle que le client a reçue (`serverInfo` et `instructions` ne comptent pas) ; sinon, ou si
  c'est une erreur, le pont s'arrête avec un message (la session du client ne peut pas être reprise telle quelle).
  Quand le serveur avait annoncé `tools.listChanged`, le client reçoit ensuite `notifications/tools/list_changed` :
  la nouvelle instance peut exposer d'autres outils. 2026-07-28 : rien n'est rejoué ; un `subscriptions/listen` en
  cours à la coupure reçoit l'erreur comme toute requête, le client le rouvre.
- **Relance de l'appli** : l'appli écrit la marque `arret-volontaire` dans son dossier de données
  (`%LOCALAPPDATA%\MCP Proxy\`) quand elle s'arrête volontairement (« Quitter », redémarrage pour une mise à jour,
  arrêt demandé par l'installation ou la désinstallation), et la retire une fois son pipe créé. Marque présente : le
  pont ne relance pas l'appli (celui qui l'a arrêtée la relance, ou personne), il l'attend. Marque absente (plantage)
  : il la relance après 10 s sans retour, une fois par coupure, et jamais deux fois en 5 min (une appli qui échoue
  aussitôt n'est pas relancée en boucle). Exe lancé : l'exe installé, par son chemin d'installation, quand le pont
  tourne depuis le dossier d'installation (sous son nom, ou renommé `mcp-proxy.exe.old`, `.new` ou en reste d'une
  mise à jour) ; sinon l'exe du pont lui-même ; toujours avec `--no-install` et détaché, comme à la spec 011. Exe à
  lancer absent pendant 2 s (désinstallation) : le pont s'arrête avec un message, sans rien relancer.
- **Premier démarrage du pont** : inchangé (spec 011) — appli lancée une fois si le pipe n'existe pas, même après un
  arrêt volontaire, attente de 15 s, preuve en échec : arrêt immédiat.
- **Arrêts** : message d'une ligne sur stderr, préfixé « MCP Proxy : », code de sortie 1. Les diagnostics du pont
  (connexion perdue et nombre de requêtes interrompues, reconnexion, preuve en échec, relance, arrêt volontaire,
  message trop long) vont aussi sur stderr, sans aucun contenu de message.
- **Compatibilité entre versions** : le pont n'ajoute rien au protocole du pipe (preuve `mcp-proxy.pipe.v1`, puis des
  lignes MCP) : un pont et une appli de versions différentes se parlent comme avant. Si une version future changeait
  la preuve, la preuve d'un pont plus ancien échouerait à chaque essai : rien n'est relayé, le pont s'arrête au bout
  de 60 s en le disant, et le client est à redémarrer (comportement d'avant cette spec). Un écart au niveau MCP est
  arrêté par la vérification du rejeu.
- **Plusieurs ponts** : chacun relance au plus une fois par coupure ; l'instance unique ne garde qu'une appli, les
  lancements suivants lui passent la main (fenêtre principale montrée, spec 011).

### Organisation du code

- `gateway::bridge::ResilientRelay`, générique sur les flux et sur une fabrique de connexion, testé sous Linux avec
  des flux en mémoire.

Réalisation : `mcp_proxy_gateway::bridge` — `ResilientRelay`, `RelayDeps`, traits `Connector` (ouverture du pipe,
`ConnectFailure`) et `AppLauncher` (`target`, `launch`), `Diagnostics`, `BridgeStop` ; module `messages`
(découpage cancel-safe, `inspect`, réponses du pont) ; module `relaunch` (`relaunch_target`, `RelaunchTarget`,
`mark_stopped_on_purpose`, `clear_stopped_on_purpose`, `stopped_marker`). `prove_and_relay` et `relay` restent
(spec 011, `EX-006-07`) mais le pont ne s'en sert plus. `app/src/bridge.rs` : pipe nommé, `Launcher` (chemins lus au
démarrage du pont), lancement détaché ; `app/src/app.rs` : marque écrite à l'arrêt, retirée après la création du
pipe. Temps de tokio : tests en horloge suspendue (`start_paused`), sans attente réelle.

## Tests

- **EX-026-01** — Coupure puis retour : le client ne voit aucune fermeture de flux.
- **EX-026-02** — Ancien protocole : `initialize` rejoué, réponse dupliquée absorbée, appels suivants servis.
- **EX-026-03** — 2026-07-28 : appels servis après reconnexion, sans rejeu.
- **EX-026-04** — Requêtes en cours à la coupure : chacune reçoit exactement une erreur.
- **EX-026-05** — Appli absente plus de 60 s : arrêt propre.
- **EX-026-06** — Messages volumineux et messages fragmentés correctement découpés.
- **EX-026-09** — Reconnexion vers un serveur qui échoue à la preuve mutuelle : aucun octet MCP ne lui est envoyé,
  ni le rejeu de `initialize`, ni les requêtes du client.
- **EX-026-10** — Requêtes du serveur qui attendaient le client à la coupure : annulées auprès du client ; sa
  réponse tardive n'est relayée à personne ; un identifiant repris par la nouvelle instance avant cette réponse est
  refusé à celle-ci et jamais montré au client.
- **EX-026-11** — Relance : l'exe installé par son chemin d'installation, jamais `.old`, `.new` ni un reste ;
  aucune relance après un arrêt volontaire ; après un plantage, une seule relance, pas avant 10 s, et pas de
  nouvelle dans les 5 min ; exe absent 2 s : arrêt sans relance, absent un instant : rien ; deux ponts : une relance
  chacun au plus, rien ne boucle ; premier démarrage du pont inchangé ; marque d'arrêt volontaire écrite et retirée.
- **EX-026-12** — Ancien protocole : une nouvelle instance qui répond une autre version du protocole, d'autres
  capacités ou une erreur arrête le pont proprement, sans rien relayer de plus ; quand `listChanged` était annoncé,
  le client reçoit `notifications/tools/list_changed` après chaque reconnexion.
- **EX-026-13** — Pendant la coupure : chaque requête reçoit l'erreur aussitôt ; un `initialized` envoyé pendant la
  coupure est rejoué ; entrée du client fermée pendant la coupure : arrêt immédiat ; fermée pendant la connexion :
  les réponses dues sont relayées, puis arrêt.
- **EX-026-14** — Les diagnostics du pont ne contiennent aucun contenu de message (paramètres, résultats) ; les
  erreurs du pont ne rendent que l'identifiant.
- **EX-026-15** — Bout en bout avec la vraie passerelle, client de l'ancien protocole et client 2026-07-28 : après
  le redémarrage (nouvelle passerelle sans secret), un appel déclenche la connexion à la demande puis est servi.

## Vérification

`cargo xtask ci` vert.

## Recette manuelle (Windows)

- **EX-026-07** (manuel) — Client LLM connecté, puis « Quitter » et relance de l'appli : le client continue de fonctionner sans être
  redémarré.
- **EX-026-08** (manuel) — Même essai avec une mise à jour suivie de « Redémarrer pour mettre à jour ».
- **EX-026-16** (manuel) — Claude Desktop et VS Code connectés, mise à jour par la fenêtre d'installation : les deux
  clients continuent sans être redémarrés, avec la nouvelle version ; l'ancienne n'est pas relancée.
- **EX-026-17** (manuel) — Désinstallation pendant qu'un client LLM tourne : son pont s'arrête en quelques secondes,
  l'appli n'est pas relancée.
- **EX-026-18** (manuel) — « Quitter » sans relancer l'appli : elle ne revient pas ; un appel pendant ce temps reçoit
  « MCP Proxy redémarre… » ; au bout de 60 s, le pont s'arrête avec son message.
- **EX-026-19** (manuel) — Plantage de l'appli (fin de tâche) avec deux clients connectés : l'appli revient seule,
  une seule instance, et les deux clients continuent.

## Amendements

- **2026-11-22** — La preuve mutuelle par DPAPI (spec 011, décision D18) est refaite à chaque reconnexion, et le
  critère `EX-026-09` est ajouté. Raison : depuis D18, le pipe n'a plus d'ACL sur mesure ni de vérification du
  binaire client ; c'est la preuve mutuelle qui protège le pont contre un pipe usurpé, et une reconnexion est
  précisément le moment où l'appli n'est pas là pour tenir le nom du pipe.
- **2026-11-23** — Statut `proposée` → `prévue`, étape 9. Raison : l'utilisateur a retenu la spec (D19), maintenant
  que l'updater (spec 020) redémarre l'appli et coupe le pont des clients LLM. Aucun critère touché.
- **2026-11-23** — Réalisation (I026.1 à I026.14). Écarts au texte : le pont ne relance l'appli qu'après un arrêt
  qu'elle n'a pas annoncé (marque `arret-volontaire` écrite à l'arrêt volontaire), après 10 s, une fois par coupure
  et pas deux fois en 5 min, toujours l'exe installé par son chemin d'installation ; exe absent : arrêt en 2 s ; les
  requêtes reçues pendant la coupure ont l'erreur aussitôt ; les requêtes du serveur vers le client sont annulées à
  la coupure ; `initialized` est repéré comme `initialize` ; la réponse rejouée doit être la même session (sinon
  arrêt) ; `notifications/tools/list_changed` après une reconnexion de l'ancien protocole. Ajouts : section
  « Réalisation », critères `EX-026-10` à `EX-026-15` (automatiques) et `EX-026-16` à `EX-026-19` (recette). Aucun
  critère retiré. Raison : les specs 019, 020 et 022, écrites après celle-ci, arrêtent et relancent l'appli
  elles-mêmes, ou retirent l'exe ; points laissés ouverts par le texte, tranchés par l'option la plus sûre.
