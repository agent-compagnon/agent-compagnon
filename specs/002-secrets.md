---
id: "002"
titre: "core — store de secrets en mémoire"
etape: "2a"
statut: "implémentée"
depend_de: ["001"]
suivi: "github#3"
---

# 002 — core — store de secrets en mémoire

## Objectif

Garder les secrets **en mémoire uniquement**, chiffrés hors usage, rattachés à une seule session, et ne jamais les
laisser sortir vers une autre destination que celles de leur session.

## À faire

### Protection mémoire

- Trait `MemoryProtector { fn protect(&self, buf: &mut [u8]) -> Result<..>; fn unprotect(&self, buf: &mut [u8]) -> Result<..>; }`.
- Windows : `CryptProtectMemory` / `CryptUnprotectMemory` avec `CRYPTPROTECTMEMORY_SAME_PROCESS`. Le tampon est
  aligné sur `CRYPTPROTECTMEMORY_BLOCK_SIZE` ; la longueur réelle est conservée à part.
- Hors Windows : `InsecureDevProtector`, sans chiffrement, compilé uniquement sous `cfg(not(windows))`, avec un
  avertissement au démarrage. Il n'existe que pour faire tourner les tests ; ne pas simuler une sécurité absente.
- Le code `unsafe` vit dans `core::platform::windows` (`#[allow(unsafe_code)]` sur ce module seulement), chaque
  bloc avec son commentaire `// SAFETY:`.

### Types

- `SealedSecret` : tampon chiffré + longueur. `Drop` efface le tampon. Ni `Clone`, ni `Serialize`.
  `Debug` affiche `SealedSecret(***)`.
- `Credential` : `Bearer { token }`, `Cookies { pairs }`, `Basic { login, password }`, construits depuis
  `secrecy::SecretString` / `SecretBox`. `Debug` masqué. Le login Basic n'est pas un secret, le mot de passe si.
- `SecretError` : aucune variante ne transporte de valeur sensible.

### `SecretStore`

- `put(&self, session: &SessionId, credential: Credential) -> Result<(), SecretError>` : remplace le secret de la
  session, scelle la valeur et ses variantes de masquage (définies en 2b). Atomique : en cas d'erreur, l'ancien
  secret reste en place.
- Validation stricte au `put`, pour rendre impossible une injection d'en-tête : valeurs secrètes non vides, 64 Kio
  au plus, sans caractère de contrôle autre que la tabulation (ni CR, ni LF, ni NUL) ; cookies : au moins un, nom
  conforme à la grammaire `token` HTTP, sans doublon, valeur sans `;` ; login Basic non vide, sans `:` ni caractère
  de contrôle. Les erreurs sont des textes fixes, jamais la valeur.
- `with_exposed<R>(&self, session, f: impl FnOnce(ExposedCredential<'_>) -> R) -> Result<R, SecretError>` :
  déchiffre dans un tampon `Zeroizing`, appelle `f`, efface. **Aucune API ne retourne la valeur en clair.**
- `with_variants<R>(&self, f: impl FnOnce(&[&[u8]]) -> R) -> Result<R, SecretError>` : expose, le temps de la
  closure, les variantes de tous les secrets actifs et des secrets retirés encore dans leur délai de grâce, pour le
  filtre. Si une variante ne peut être déchiffrée, la closure n'est pas appelée : le filtre échoue fermé.
- `wipe(session) -> bool` (vrai s'il y avait un secret), `wipe_all()`, `has(session)` (secret actif seulement).
- **Variantes retirées.** Après `wipe`, `wipe_all` ou le remplacement d'un secret par `put`, l'ancien secret n'est
  plus injectable (`with_exposed` échoue, `has` est faux), mais ses variantes restent scellées et connues du filtre
  pendant un délai de grâce : une réponse à un appel parti avant l'effacement peut encore le contenir. Passé ce
  délai, elles sont effacées à la première opération du store. Un `put` sur la même session ne raccourcit pas le
  délai des variantes déjà retirées. Le délai est un paramètre du store (`with_variant_grace`, défaut
  `DEFAULT_VARIANT_GRACE` = 130 s, câblé plus tard sur `upstream_timeout_secs` + 10 s) ; l'horloge est injectable
  (`with_clock`, trait `Clock`).
- Un secret appartient à une session et une seule.

### Liaison secret ↔ destination

- `OriginGuard` : pour chaque session, l'ensemble des origines autorisées (schéma + hôte + port), dérivé de la
  config : URL des instances MCP qui utilisent la session, `base_url` des modules locaux, `probe_url`.
- `authorize(session, &Url) -> Result<(), SecretError>` : refuse une session inconnue, une URL avec identifiants,
  toute autre origine et tout schéma non `https`. `http` n'est accepté que pour la boucle locale (`localhost` et
  `127.0.0.1`, pour les tests), et seulement si l'origine est déclarée.

### Client HTTP authentifié

- `AuthenticatedHttpClient`, construit par le core pour une session. C'est ce que reçoivent les modules locaux.
- À chaque requête : `authorize`, puis injection des en-têtes au moment de l'envoi via `with_exposed`
  (`Authorization: Bearer …`, `Cookie: …`, `Authorization: Basic …`).
- L'appelant ne peut lire ni l'en-tête injecté ni le secret ; les en-têtes `Authorization` et `Cookie` qu'il aurait
  posés sont retirés. Les en-têtes de réponse `Set-Cookie`, `Authorization`, `WWW-Authenticate`,
  `Proxy-Authenticate` sont retirés avant de rendre la réponse, ainsi que `Proxy-Authorization` et `Set-Cookie2`.
- Aucune redirection suivie (`redirect::Policy::none()`), `Referer` désactivé. TLS natif, vérification jamais
  désactivée. Les erreurs de transport sont réduites à `Timeout`, `Connect` ou `Transport`, sans l'URL.
- Deux clients internes : celui des origines distantes garde les proxys de l'environnement ; les requêtes vers la
  boucle locale passent par un second client sans aucun proxy, pour qu'un en-tête d'authentification en `http`
  clair ne soit jamais remis à un proxy.
- Délai par requête configurable : celui du client borne chaque requête, connexion comprise ; une requête peut
  demander un délai plus court, jamais plus long.

## Tests

- **EX-002-01** — Aller-retour `protect` / `unprotect`. Tests `#[cfg(windows)]` pour `CryptProtectMemory`, compilés en contrôle
  croisé.
- **EX-002-02** — Aucune valeur sensible dans `Debug` ni `Display`, pour tous les types, avec une valeur sentinelle. Version
  property-based (proptest) : quelle que soit la valeur, elle n'apparaît dans aucune sortie.
- **EX-002-03** — `put` remplace ; `wipe` vide la session ; `with_exposed` sur une session vide renvoie une erreur sans détail
  sensible.
- **EX-002-04** — Après `Drop`, le tampon est à zéro (accès de test dédié).
- **EX-002-05** — `authorize` : autre hôte, autre port, `http` refusés ; origine déclarée acceptée.
- **EX-002-06** — `AuthenticatedHttpClient` contre un serveur local : en-tête présent côté serveur ; redirection vers une autre
  origine non suivie ; `Set-Cookie` absent de la réponse rendue ; requête vers une origine non déclarée refusée
  **avant** tout envoi.
- **EX-002-10** — Après `wipe`, `wipe_all` ou un remplacement par `put`, le secret n'est plus injectable mais ses
  variantes restent exposées par `with_variants` pendant le délai de grâce, y compris après un nouveau `put` sur la
  même session ; une fois le délai écoulé (horloge de test), elles ont disparu et leurs tampons sont à zéro.

## Recette manuelle (Windows)

Le code Windows n'est que compilé sous Linux ; ces critères se vérifient sur un poste Windows (`docs/recette.md`).

- **EX-002-07** (manuel) — `cargo test -p mcp-proxy-core platform::windows` : les 2 tests passent (aller-retour
  `CryptProtectMemory` / `CryptUnprotectMemory` avec un tampon effectivement modifié par le chiffrement ; tampon non
  aligné refusé).
- **EX-002-08** (manuel) — `cargo test -p mcp-proxy-core secrets` : tous les tests passent ;
  `platform_store_round_trip` y utilise `WindowsMemoryProtector` et les tests du client HTTP passent par SChannel.
- **EX-002-09** (manuel) — Aucun message « secrets en mémoire NON chiffrés » dans la sortie des tests : le
  protecteur de développement n'existe pas sous Windows.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Calcul des variantes de masquage et filtre (2b). Acquisition des secrets (étape 4).

## Amendements

- **2026-11-22** — Rattrapage après implémentation (spec 030). Le texte décrit désormais ce qui a été livré :
  - `with_variants` renvoie un `Result` et n'appelle pas la closure si une variante est illisible (I2a.6) ;
  - `put` renvoie un `Result`, est atomique et valide strictement les valeurs pour empêcher toute injection
    d'en-tête (I2a.7, I2a.8) ;
  - `Proxy-Authorization` et `Set-Cookie2` sont aussi retirés des réponses, les en-têtes d'authentification posés
    par l'appelant sont retirés de la requête (I2a.12) ;
  - `http` n'est toléré que pour la boucle locale déclarée (`localhost`, `127.0.0.1`), comme pour la config
    (I2a.11) ;
  - deux clients internes, le second sans proxy pour la boucle locale (I2a.14) ; délai du client comme plafond
    (I2a.13) ;
  - les trois vérifications Windows de l'étape 2a deviennent les critères manuels `EX-002-07` à `EX-002-09`.
- **2026-11-22** — Variantes retirées (relecture de l'étape 2a, avant la spec 003). Jusqu'ici, `wipe` faisait
  disparaître les variantes du secret en même temps que le secret : une réponse encore en route (appel parti avant
  une déconnexion ou une expiration) qui contiendrait ce secret ne serait plus masquée. Désormais, après `wipe`,
  `wipe_all` ou un remplacement par `put`, le secret n'est plus injectable mais ses variantes restent scellées et
  connues du filtre pendant un délai de grâce, puis sont effacées. Délai paramétrable, horloge injectable. Nouveau
  critère `EX-002-10` (I002.1 à I002.3).
