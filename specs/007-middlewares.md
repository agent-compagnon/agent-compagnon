---
id: "007"
titre: "gateway — chaîne de middlewares"
etape: "3c"
statut: "implémentée"
depend_de: ["003", "004", "006"]
suivi: "github#8"
---

# 007 — gateway — chaîne de middlewares

## Objectif

Faire passer chaque `tools/call` par la chaîne complète : politique, scan des arguments, limite de débit,
confirmation, session, injection, filtre de sortie, trace.

## À faire

### Structure

- `CallContext { call_id, client, tool, args, deadline, policy, .. }` et un trait `Middleware` ; la chaîne est
  assemblée une fois et partagée.
- `deadline` = maintenant + `call_budget_secs` (50 s). C'est le temps maximal passé à attendre **l'utilisateur**.
- Réalisé : `mcp_proxy_gateway::server::CallChain`, implémentation de `CallPipeline` construite par
  `CallChain::new(CallChainDeps { config, registry, settings, upstreams, gate, broker, output, trace })`.
  `Middleware { name, handle(&mut CallContext) -> Result<(), Halt> }` ; `Halt` vaut `Refused(Refusal { status,
  message })` ou `InputRequired(..)`. Le premier `Err` arrête la chaîne : aucun maillon suivant ne s'exécute.
  L'ordre est fixe, exposé par `STAGES` et `CallChain::stage_names`. Les réglages (suspension, profil,
  durcissement) sont lus à chaque appel dans un `watch::Receiver<UserSettings>`.
- Le filtre de sortie et la trace ne sont pas des `Middleware` : ils suivent toujours la chaîne, quelle que soit
  l'issue, refus compris. La réponse est toujours construite par `OutputGate` (INV-3), y compris un
  `input_required`.

### Maillons, dans l'ordre

1. **Résolution et exposition.** Outil non exposé → refus. Passerelle suspendue (5e) → refus.
2. **Politique.** `PolicyEngine::resolve`. `Deny` → refus immédiat, avec l'explication de la règle.
3. **Scan des arguments.** `OutputFilter::contains_known_secret(args)` → appel bloqué. Le message ne répète rien
   des arguments.
4. **Limite de débit.** Seau à jetons par instance (`per_minute`, `burst`). Refus avec le délai avant réessai.
   Aucun jeton consommé si l'appel a été refusé plus haut.
5. **Confirmation**, si la politique l'exige.
6. **Session.** `SessionGate::ensure` avec `min(deadline, auth_timeout_secs)`. Délai dépassé → erreur « réessayez »,
   la connexion restant en cours.
7. **Appel amont.** Sur `AuthRejected` : session marquée expirée, nouvel `ensure` dans le budget restant, rejeu
   **une seule fois**.
8. **Filtre de sortie** sur le résultat ou l'erreur. Si la liste noire a trouvé quelque chose, statut
   `leak_masked`.
9. **Trace.** Émission d'un `CallRecord` vers un trait `TraceSink` (implémentation réelle en 3e).

Précisions de réalisation :

- La suspension est un maillon à part, juste après la résolution : `resolution`, `suspension`, `policy`,
  `arguments`, `rate_limit`, `confirmation`, `session`, `upstream`. Un nom inconnu est refusé comme tel même
  passerelle suspendue ; un appel refusé pour suspension ne résout aucune politique.
- Catégorie pour la politique : annotations du serveur gardées par le registre, utilisées seulement pour un outil
  déclaré sans catégorie (S11).
- `ensure` est borné par la chaîne elle-même (`timeout_at`), quoi que fasse le `SessionGate` : à l'échéance, message
  « connexion requise … réessayez ». Une session sans secret, ou dont la connexion est annulée ou impossible, donne
  son propre message.
- Le rejeu suppose que l'amont a déjà marqué la session rejetée et effacé son secret avant de rendre
  `AuthRejected` (contrat de `Upstream`, tenu par `HttpUpstream`) : la chaîne ne rappelle pas `mark_rejected`. Un
  second `AuthRejected` est une erreur (`auth_required`), jamais un troisième essai.
- Annulation par le client : à tout moment d'attente (confirmation, connexion, appel amont), statut `cancelled`.

### Confirmation

```rust
#[async_trait]
pub trait ConfirmationBroker: Send + Sync {
    async fn confirm(&self, request: ConfirmationRequest) -> ConfirmationOutcome; // Approved | Refused | TimedOut | Unavailable
}
```

- Choix du canal : la politique dit `Elicitation` **et** le client l'annonce **et** son nom figure dans
  `elicitation_clients` → elicitation. Sinon → `ConfirmationBroker` (notification Windows, 5c). Si la politique dit
  `Notification`, l'elicitation n'est jamais utilisée.
- La demande montre le chemin (`PROD / kibana-logs / delete_index`), le client et les arguments **filtrés**.
- Délai `confirmation_timeout_secs`. Sans réponse : refus. Seule une acceptation explicite autorise l'appel.
- Elicitation, ancien protocole : `peer.create_elicitation(..)` avec délai. `accept` autorise ; `decline`,
  `cancel`, erreur ou délai refusent.
- Elicitation, 2026-07-28 : renvoyer `CallToolResponse::InputRequired` avec un `request_state` signé par
  `RequestStateCodec` (clé aléatoire par processus). L'état lie : connexion, nom exposé, SHA-256 des arguments
  canoniques, nonce, expiration. Au rejeu : signature et expiration valides, mêmes arguments, nonce jamais vu puis
  consommé. Toute incohérence → refus.
- Si l'elicitation échoue pour une raison technique, repli sur la notification Windows dans le délai restant.

Précisions de réalisation :

- `ConfirmationRequest` gagne `path` (`DEV / kibana-logs / delete_index`). Le nom du client et les arguments y sont
  filtrés ; si le filtre est indisponible, rien n'est demandé et l'appel est refusé.
- Délai : `min(confirmation_timeout_secs, temps restant du budget)`, borné par la chaîne autour du
  `ConfirmationBroker` comme de l'elicitation. Le message de délai cite ce délai effectif.
- Le nom du client doit figurer **exactement** (casse comprise) dans `elicitation_clients`.
- Elicitation : formulaire sans champ (`requestedSchema` objet vide) ; seule l'action `accept` autorise, son contenu
  est ignoré. Le message (chemin, client, arguments filtrés, explication de la règle) passe par le filtre.
- Ancien protocole : `create_elicitation_with_timeout`. Erreur JSON-RPC renvoyée par le client ou réponse
  inattendue : refus. Seule une panne de transport (envoi impossible, connexion fermée) est une « raison
  technique » qui donne le repli sur le `ConfirmationBroker`.
- 2026-07-28 : `input_required` avec une seule demande, clé `confirmation`. Premier tour : ni connexion ni appel
  amont, statut `input_required`. Au second tour, `input_responses` doit contenir exactement cette clé et un
  `ElicitResult` lisible. L'état (`rs1`, données associées `mcp-proxy.confirmation.v1`, durée de vie
  `confirmation_timeout_secs`) porte connexion, nom exposé, SHA-256 du JSON canonique des arguments, nonce de 128
  bits et expiration ; le nonce est gardé côté passerelle jusqu'à son usage ou son expiration (1024 en attente au
  plus) et **brûlé dès qu'un état bien signé revient**, même si le reste est incohérent. Un état expiré donne le
  message de délai ; tout autre défaut, « confirmation invalide ou expirée ». Un état ou des réponses présentés
  alors que la chaîne n'attend pas cette confirmation sont refusés. Une impossibilité d'émettre l'état (aléa
  indisponible) est la raison technique qui donne le repli sur le `ConfirmationBroker`.
- Connexion : identifiant unique dans le processus, attribué par `GatewayServer::serve` à chaque flux client.
- Chaque tour d'un appel 2026-07-28 passe la limite de débit : un appel confirmé dans le chat consomme deux jetons.

### Messages renvoyés au LLM

| Cas | Message |
|---|---|
| Politique | `MCP Proxy : appel interdit par la politique (PROD/kibana-logs/delete_index).` |
| Secret dans les arguments | `MCP Proxy : appel bloqué, les arguments contiennent un secret connu.` |
| Débit | `MCP Proxy : limite de débit atteinte pour Kibana Logs (PROD). Réessayez dans 12 s.` |
| Refus | `MCP Proxy : appel refusé par l'utilisateur.` |
| Délai de confirmation | `MCP Proxy : confirmation non reçue dans le délai (30 s), appel refusé.` |
| Connexion en attente | `MCP Proxy : connexion à « SSO PROD » requise. La fenêtre de connexion est ouverte sur le poste de l'utilisateur. Réessayez une fois la connexion faite.` |
| Suspension | `MCP Proxy : passerelle suspendue par l'utilisateur.` |

Messages ajoutés à la réalisation (amendement du 2026-11-22) :

| Cas | Message |
|---|---|
| Politique (forme réelle) | `MCP Proxy : appel interdit par la politique (PROD/kibana-logs/delete_index). Raison : <explication de la règle>.` |
| Outil inconnu ou non exposé | `MCP Proxy : outil « <nom> » inconnu ou non exposé, appel refusé.` (spec 006) |
| Aucun profil applicable | `MCP Proxy : aucun profil de politique applicable, appel refusé.` |
| Filtre indisponible pour le scan | `MCP Proxy : appel bloqué, le filtre est indisponible pour vérifier les arguments.` |
| Confirmation invalide | `MCP Proxy : confirmation invalide ou expirée, appel refusé.` |
| Confirmation impossible | `MCP Proxy : confirmation impossible, aucun moyen de la demander à l'utilisateur ; appel refusé.` |
| Filtre indisponible pour la confirmation | `MCP Proxy : confirmation impossible, le filtre est indisponible ; appel refusé.` |
| Connexion annulée | `MCP Proxy : connexion à « SSO PROD » annulée par l'utilisateur, appel refusé.` |
| Connexion impossible | `MCP Proxy : connexion à « SSO PROD » impossible pour le moment, appel refusé.` |
| Identifiant refusé deux fois | `MCP Proxy : prod/kibana-logs : identifiant refusé par le serveur, même après une nouvelle connexion ; appel abandonné.` |
| Annulation | `MCP Proxy : appel annulé par le client.` |
| Erreur amont | `MCP Proxy : prod/kibana-logs : <erreur>` (spec 006) |

### Trace

- `mcp_proxy_core::trace` : `CallRecord { id, started_at, duration, client, tool, env, mcp, session, policy,
  status, request, response, filter_report }`, `PolicyRecord { decision, explanation }`, `CallStatus`, trait
  `TraceSink { record(CallRecord) }` (synchrone, ne doit pas bloquer), `NoTrace`. Le testkit fournit
  `MemoryTraceSink`.
- Contenu déjà filtré : nom du client, nom demandé, explication, arguments, réponse envoyée. `env`, `mcp`,
  `session` sont connus après la résolution, `policy` après la politique.
- Statuts : ceux de la spec 009 (`ok`, `leak_masked`, `error`, `denied`, `refused`, `timeout`, `blocked`,
  `rate_limited`, `auth_required`), plus `suspended`, `input_required` (premier tour d'une confirmation dans le
  chat) et `cancelled`. `leak_masked` remplace `ok` ou `error` dès que la liste noire a masqué quelque chose dans la
  réponse amont (résultat ou erreur). Outil inconnu : `denied`. Confirmation impossible ou invalide : `refused`.

## Tests

De bout en bout, avec le testkit :

- **EX-007-01** — politique `Deny` ; `Confirm` approuvé, refusé, sans réponse ;
- **EX-007-02** — secret connu dans les arguments, sous plusieurs encodages : bloqué, aucun appel amont ;
- **EX-007-03** — limite de débit atteinte puis rétablie ;
- **EX-007-04** — en-tête injecté uniquement vers l'origine de la session ;
- **EX-007-05** — secret renvoyé par le serveur : masqué, statut `leak_masked` ;
- **EX-007-06** — 401 → reconnexion → rejeu réussi ; second 401 → erreur, pas de boucle ;
- **EX-007-07** — budget dépassé pendant une connexion : message « réessayez », puis succès à l'appel suivant ;
- **EX-007-08** — elicitation ancienne génération : accepté, refusé, délai ;
- **EX-007-09** — elicitation 2026-07-28 : aller-retour complet ; `request_state` altéré, expiré ou rejoué refusé ; arguments
  modifiés entre les deux tours refusés ;
- **EX-007-10** — client absent de `elicitation_clients` : la notification est utilisée ;
- **EX-007-11** — chaque refus produit un `CallRecord` au bon statut, au contenu déjà filtré.
- **EX-007-12** — passerelle suspendue : tout appel est refusé avec le message prévu, sans politique, confirmation,
  connexion ni appel amont ; la résolution reste la première étape ;
- **EX-007-13** — ordre des maillons : celui de la spec ; un appel refusé par la politique ou bloqué au scan ne
  consomme aucun jeton de débit et ne déclenche ni confirmation ni connexion.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Affichage des confirmations Windows (5c). Écriture des traces (3e).

## Amendements

- **2026-11-22** — Réalisation. Ajouts : précisions de structure, d'ordre (suspension en maillon séparé juste après
  la résolution), de confirmation (canal, délais, état signé, repli) et de trace ; champ `path` de
  `ConfirmationRequest` ; messages ajoutés ; statuts `suspended`, `input_required` et `cancelled` en plus de ceux de
  la spec 009 ; critères `EX-007-12` (suspension) et `EX-007-13` (ordre et jetons), qui n'étaient décrits que dans
  le texte. Le message de politique ajoute l'explication de la règle, comme le demande le maillon 2. Raison : points
  laissés ouverts par le texte, tranchés par l'option la plus stricte (décisions I007.x).
