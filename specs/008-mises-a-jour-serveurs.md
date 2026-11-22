---
id: "008"
titre: "gateway — mises à jour des serveurs MCP"
etape: "3d"
statut: "en cours"
depend_de: ["005", "006", "007"]
suivi: "github#9"
---

# 008 — gateway — mises à jour des serveurs MCP

## Objectif

Suivre l'évolution des outils des serveurs : relayer les changements, recharger les listes, détecter les
différences, et traiter tout outil nouveau avec prudence.

## À faire

### Sources de changement

- Ancien protocole : `ClientHandler::on_tool_list_changed`.
- 2026-07-28 : abonnement `Peer::listen(SubscriptionFilter { tools_list_changed: true, .. })` tenu ouvert tant que
  l'instance est connectée ; `ttl_ms` des listes respecté.
- Serveurs qui ne notifient pas : rechargement toutes les `refresh_interval_secs` (300 s), pour les instances
  connectées ou dont la liste est anonyme.
- À chaque connexion et reconnexion.

Précisions de réalisation (amendement du 2026-11-22) :

- `mcp_proxy_gateway::updates::ListWatcher` : une tâche par amont, construite par `ListWatcher::spawn(ListWatcherDeps
  { config, registry, upstreams, gate, settings, output, tracker, timing })`, arrêtée quand le `ListWatcher` est
  relâché. `WatchTiming { refresh_interval, debounce, min_refresh }` : `refresh_interval_secs`, 500 ms d'anti-rebond,
  30 s au plus court entre deux rechargements dictés par `ttl_ms`.
- Le trait `Upstream` gagne `connections()` (signal après chaque connexion ou reconnexion), `is_connected()` (sans
  jamais ouvrir de connexion) et `list_ttl()` (`ttl_ms` de la dernière liste complète), avec des valeurs par défaut
  pour les amonts sans connexion.
- Le cache de réponses du client `rmcp` est **désactivé** pour chaque connexion amont : chaque rechargement relit le
  serveur, et un échec n'est jamais masqué par une liste périmée (`serve_stale_on_error`, actif par défaut dans
  `rmcp`).
- `ttl_ms` respecté : une liste dont `ttl_ms` est plus court que `refresh_interval_secs` est rechargée à son
  expiration, jamais plus souvent que toutes les 30 s. `ttl_ms = 0`, valeur que `rmcp` envoie pour un serveur
  2026-07-28 qui n'en donne pas (« périmée tout de suite »), ne raccourcit rien.
- 2026-07-28 : l'abonnement fermé alors que la connexion vit (serveur redémarré, flux coupé) est rouvert après
  `initial_backoff` (1 s), puis un changement est signalé, puisqu'une notification a pu être perdue entre-temps.
- Reconnexion : nouvelle connexion de `HttpUpstream` et, en ancien protocole, toute réinitialisation transparente
  d'une session expirée par le transport `rmcp` (un `initialize` accepté), qui suit typiquement un redémarrage du
  serveur.
- Rechargement périodique : au démarrage du `ListWatcher`, puis à chaque période, pour les instances dont le MCP
  n'est pas coupé et qui sont soit connectées, soit **exposées** avec une liste qui ne demande aucune connexion de
  l'utilisateur (liste anonyme, ou session déjà active). Une session expirée n'est jamais reconnectée par un
  rechargement : `SessionGate::ensure` n'est pas appelé, l'instance passe à `SessionRequired`.
- MCP coupé par l'utilisateur : aucun rechargement, quelle que soit la source ; état `Disabled`.

### Traitement d'un changement

1. Recharger `tools/list`.
2. `registry.apply_live_list` : préfixes, catégories et politiques sont réappliqués, l'empreinte comparée.
3. Empreinte inchangée : rien d'autre, aucune notification.
4. Sinon : cache mis à jour, puis notification des clients connectés — `Peer::notify_tool_list_changed` pour les
   clients en ancien protocole, `SubscriptionSink::notify_tool_list_changed` pour ceux qui ont ouvert un
   `subscriptions/listen`.
5. Événement `ToolsChanged` pour l'interface.

- Rafales de notifications regroupées (anti-rebond).
- Rechargement en échec : la dernière liste connue est conservée, l'instance passe à l'état `Unreachable`.

Précisions de réalisation : la notification des clients est celle de la spec 006 (`EX-006-10`), déclenchée par
l'événement `ToolsChanged` du registre, qui n'est émis que si la différence n'est pas vide ; une empreinte
inchangée n'écrit pas le cache (spec 005 : seule la première liste réelle qui remplace un instantané embarqué
l'écrit). Anti-rebond : la première notification ou connexion ouvre une fenêtre de 500 ms ; tout ce qui arrive
pendant la fenêtre est absorbé, et un seul rechargement suit. Une notification qui arrive pendant un rechargement en
provoque un autre, la liste ayant pu être lue avant le changement. Un rechargement échoue aussi quand le registre
refuse la liste (spec 005) : état `Unreachable`. `SessionRequired`, `AuthRejected` et les erreurs de session donnent
`SessionRequired`. Le message d'échec gardé pour l'interface passe par le filtre de sortie.

### Outils nouveaux et modifiés

- Un outil absent de la config est `known = false`. Il démarre en **confirmation manuelle par notification
  Windows**, ou en **interdit**, selon `unknown_tools` du profil (2c), et il est traité comme destructif.
- `McpUpdate { instance, added, modified, removed, at }` reste en attente jusqu'à `acknowledge(instance)`. Il
  alimente le bandeau « MCP mis à jour, N nouveaux outils » (6b).
- Un outil connu dont la description, le schéma ou les annotations changent est signalé « modifié » dans le journal
  et le bandeau. Sa politique ne change pas : c'est la config qui décide.

Précisions de réalisation : `McpUpdate` est cumulé, par instance, depuis le dernier acquittement : un outil ajouté
puis retiré ne laisse rien ; un outil retiré puis revenu compte comme modifié ; un outil ajouté puis modifié reste
nouveau. `at` est l'heure du dernier changement cumulé.

### États d'une instance

`Catalogue` (jamais connectée, outils issus du catalogue), `Connected`, `SessionRequired`, `Unreachable`,
`Disabled`. Exposés à l'interface.

Précisions de réalisation : `UpdateTracker` (partagé, clonable) porte, pour chaque instance de la config,
`InstanceStatus { state, refreshed_at, last_error }` et la mise à jour en attente : `state`, `status`, `statuses`,
`pending`, `pending_for`, `acknowledge`, `subscribe`. Événements (`UpdateEvent`) : `StateChanged`, `Reloaded {
outcome }` (`Unchanged`, `Changed(diff)`, `Failed(état)`), `UpdatePending`, `Acknowledged`. `Disabled` se superpose
à l'état du dernier rechargement, qui revient quand le MCP est rétabli. `InstanceState::label` donne le libellé
français. `ListWatcher::reload(instance)` demande un rechargement immédiat (bouton de l'interface).

## Tests

Avec le faux serveur :

- **EX-008-01** — notification en ancien protocole : liste rechargée, clients notifiés ;
- **EX-008-02** — abonnement 2026-07-28 : idem ;
- **EX-008-03** — serveur muet : la différence est détectée au rechargement périodique (horloge de test) ;
- **EX-008-04** — reconnexion : liste rechargée ;
- **EX-008-05** — empreinte inchangée : aucune notification envoyée aux clients ;
- **EX-008-06** — nouvel outil : inconnu, confirmation ou interdit selon le profil, jamais autorisé d'office ;
- **EX-008-07** — description modifiée : signalée, politique inchangée ;
- **EX-008-08** — outil supprimé : retiré de la liste exposée, appel refusé ;
- **EX-008-09** — rechargement en échec : liste conservée, état `Unreachable` ;
- **EX-008-10** — `acknowledge` vide la mise à jour en attente.
- **EX-008-11** — `ttl_ms` plus court que la période : liste relue sur le serveur à son expiration, pas avant ;
- **EX-008-12** — rafale de notifications : un seul rechargement (horloge de test) ;
- **EX-008-13** — rechargement périodique sans connexion de l'utilisateur : instance jamais connectée sans session,
  aucun contact ni `ensure` ; session expirée, état `SessionRequired` sans `ensure` ; liste anonyme exposée,
  rechargée sans identifiant ;
- **EX-008-14** — états exposés pour chaque instance (`Catalogue`, `Connected`, `Disabled`) ; MCP coupé : ni
  notification ni période ne contactent son serveur, l'état d'avant revient quand il est rétabli.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Bandeau dans l'interface (6b).

## Amendements

- **2026-11-22** — Réalisation (I008.1 à I008.12). Ajouts : précisions de réalisation dans chaque section
  (`ListWatcher`, `UpdateTracker`, cache client de `rmcp` désactivé, réabonnement 2026-07-28, réinitialisation de
  session comptée comme reconnexion, anti-rebond, cumul de `McpUpdate`, état `Disabled`) ; critères `EX-008-11` à
  `EX-008-14`, décrits dans le texte sans critère jusqu'ici. Écarts : `ttl_ms = 0` ne raccourcit pas la période, et
  un rechargement dicté par `ttl_ms` n'a jamais lieu moins de 30 s après le précédent ; le rechargement périodique
  d'une instance non connectée est réservé aux instances exposées, et vaut aussi pour une session déjà active (ce qui
  ne déclenche aucune connexion de l'utilisateur) ; une liste refusée par le registre met l'instance en
  `Unreachable`. Raison : points laissés ouverts par le texte, tranchés par l'option la plus sûre ou la plus sobre
  en trafic, et comportement réel de `rmcp` 3.11 constaté dans ses sources.
