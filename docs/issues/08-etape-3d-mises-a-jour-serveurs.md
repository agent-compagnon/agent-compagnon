# [Étape 3d] gateway — mises à jour des serveurs MCP

> Référence commune : `00-cadrage.md`. Dépend de : étapes 3a, 3b, 3c.

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

### Outils nouveaux et modifiés

- Un outil absent de la config est `known = false`. Il démarre en **confirmation manuelle par notification
  Windows**, ou en **interdit**, selon `unknown_tools` du profil (2c), et il est traité comme destructif.
- `McpUpdate { instance, added, modified, removed, at }` reste en attente jusqu'à `acknowledge(instance)`. Il
  alimente le bandeau « MCP mis à jour, N nouveaux outils » (6b).
- Un outil connu dont la description, le schéma ou les annotations changent est signalé « modifié » dans le journal
  et le bandeau. Sa politique ne change pas : c'est la config qui décide.

### États d'une instance

`Catalogue` (jamais connectée, outils issus du catalogue), `Connected`, `SessionRequired`, `Unreachable`,
`Disabled`. Exposés à l'interface.

## Tests

Avec le faux serveur :

- notification en ancien protocole : liste rechargée, clients notifiés ;
- abonnement 2026-07-28 : idem ;
- serveur muet : la différence est détectée au rechargement périodique (horloge de test) ;
- reconnexion : liste rechargée ;
- empreinte inchangée : aucune notification envoyée aux clients ;
- nouvel outil : inconnu, confirmation ou interdit selon le profil, jamais autorisé d'office ;
- description modifiée : signalée, politique inchangée ;
- outil supprimé : retiré de la liste exposée, appel refusé ;
- rechargement en échec : liste conservée, état `Unreachable` ;
- `acknowledge` vide la mise à jour en attente.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Bandeau dans l'interface (6b).
