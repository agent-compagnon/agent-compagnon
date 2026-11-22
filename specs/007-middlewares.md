---
id: "007"
titre: "gateway — chaîne de middlewares"
etape: "3c"
statut: "prévue"
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

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Affichage des confirmations Windows (5c). Écriture des traces (3e).
