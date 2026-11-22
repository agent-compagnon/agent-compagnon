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

### Protection mémoire : enveloppe

Aucun `unsafe` (D18) : la protection est une **enveloppe** faite d'un AEAD de RustCrypto et d'une clé gardée
enveloppée par DPAPI.

- Une **clé de session** aléatoire de 256 bits (générateur du système, `getrandom`) est créée avec le store.
- Chaque valeur secrète et chaque variante est scellée par XChaCha20-Poly1305 (`chacha20poly1305`), avec un nonce
  aléatoire de 192 bits par scellement et, en données associées, l'identifiant de la session et la place de la
  valeur (`bearer`, `cookie:<nom>`, `basic:<login>`, `variant`) : un secret scellé pour une session ne s'ouvre pas
  pour une autre, et une altération est détectée à l'ouverture (`Corrupted`).
- La clé de session n'est **jamais conservée en clair** : le store ne garde que ce qu'a rendu
  `KeyWrapper::wrap(&[u8]) -> Result<Vec<u8>, _>`, et l'ouvre avec `KeyWrapper::unwrap(&[u8]) ->
  Result<Zeroizing<Vec<u8>>, _>`.
- Windows : `DpapiKeyWrapper`, DPAPI au niveau utilisateur (descripteur `LOCAL=user`) par la projection WinRT du
  crate `windows` (`DataProtectionProvider`, `CryptographicBuffer`). L'enrobage vit dans `core::platform::windows`
  (`protect(&[u8])`, `unprotect(&[u8])`), sûr et réutilisable : les specs 011 et 014 s'en servent.
- Hors Windows : `InsecureDevKeyWrapper`, qui garde la clé telle quelle, compilé uniquement sous
  `cfg(not(windows))`, avec un avertissement à sa construction. Il n'existe que pour faire tourner les tests ; ne
  pas simuler une sécurité absente.
- **Un déballage par usage** : `put`, `with_exposed` et `with_variants` déballent la clé une seule fois, quel que
  soit le nombre de valeurs, scellent ou déchiffrent, puis effacent la clé **avant** d'appeler la closure ; les
  clairs sont effacés quand elle rend la main. Aucun appel DPAPI quand la session n'a pas de secret.
- **Échec fermé** : si le déballage échoue, l'opération renvoie une erreur et la closure n'est pas appelée ; un
  `put` qui échoue laisse l'ancien secret en place. `SecretStore::new` (donc `SecretStore::for_platform`) fait un
  aller-retour d'essai de la clé et renvoie `KeyProtectionUnavailable` si l'enrobage ne répond pas ou rend autre
  chose ; il n'y a pas de repli vers un mode plus faible.

### `SecretStore`

- `put(&self, session: &SessionId, credential: Credential) -> Result<(), SecretError>` : remplace le secret de la
  session, scelle la valeur et ses variantes de masquage (définies en 2b). Atomique : en cas d'erreur, l'ancien
  secret reste en place.
- Validation stricte au `put`, pour rendre impossible une injection d'en-tête : valeurs secrètes non vides, 64 Kio
  au plus, sans caractère de contrôle autre que la tabulation (ni CR, ni LF, ni NUL) ; cookies : au moins un, nom
  conforme à la grammaire `token` HTTP, sans doublon, valeur sans `;` ; login Basic non vide, sans `:` ni caractère
  de contrôle. Les erreurs sont des textes fixes, jamais la valeur.
- `with_exposed<R>(&self, session, f: impl FnOnce(ExposedCredential<'_>) -> R) -> Result<R, SecretError>` :
  déballe la clé, déchiffre dans un tampon `Zeroizing`, efface la clé, appelle `f`, efface. **Aucune API ne retourne la valeur en clair.**
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
  (`Authorization: Bearer …`, `Cookie: …`, `Authorization: Basic …`). `new(session, store, guard, timeout)` injecte
  par un `StoreSessionGate` ; `through_gate(session, gate, timeout)` injecte par le `SessionGate` donné (spec 025).
- Spec 025 : `only_towards(origine)` restreint le client à une seule origine (jamais élargi) ; `TRACE` et `CONNECT`
  sont refusés avant tout envoi ; un 401 ou un 403 marque refusé le secret envoyé (`SessionGate::mark_rejected`) et
  incrémente le `RejectionCounter` rendu par `through_gate`.
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

- ~~**EX-002-01** — Aller-retour `protect` / `unprotect`. Tests `#[cfg(windows)]` pour `CryptProtectMemory`, compilés
  en contrôle croisé.~~ (retiré le 2026-11-22 : D18, plus de `CryptProtectMemory` ; remplacé par `EX-002-11`)
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
- **EX-002-11** — Aller-retour de l'enveloppe : scellement puis ouverture pour des valeurs de 0 octet à
  `MAX_SECRET_LEN`, l'ouverture travaillant sur une copie ; deux scellements de la même valeur diffèrent (nonce
  aléatoire) et ne contiennent pas la valeur ; enrobage de développement et store de la plateforme en aller-retour.
- **EX-002-12** — Altération détectée : un bit changé dans un secret scellé ou dans une variante fait échouer
  l'ouverture (`Corrupted`) ; dans le store, `with_exposed` et `with_variants` échouent sans appeler la closure.
- **EX-002-13** — Liaison à la session : une valeur scellée pour une session ne s'ouvre ni pour une autre session,
  ni pour une autre place, ni avec la clé d'un autre store ; un secret déplacé sous une autre session dans le store
  ne s'ouvre pas.
- **EX-002-14** — Un seul déballage par usage (enrobage de test qui compte) : un pour `put`, un pour `with_exposed`
  quel que soit le nombre de cookies, un pour `with_variants` quel que soit le nombre de variantes et de sessions,
  aucun pour une session sans secret ; la clé déballée est à zéro avant l'appel de la closure.
- **EX-002-15** — Échec fermé : si le déballage échoue, `with_exposed` et `with_variants` renvoient une erreur sans
  appeler la closure et `put` échoue en laissant l'ancien secret ; la création d'un store échoue avec
  `KeyProtectionUnavailable` si l'enrobage échoue ou si l'aller-retour d'essai ne rend pas la clé.

## Recette manuelle (Windows)

Le code Windows n'est que compilé sous Linux ; ces critères se vérifient sur un poste Windows (`docs/recette.md`).

- ~~**EX-002-07** (manuel) — `cargo test -p mcp-proxy-core platform::windows` : les 2 tests passent (aller-retour
  `CryptProtectMemory` / `CryptUnprotectMemory` avec un tampon effectivement modifié par le chiffrement ; tampon non
  aligné refusé).~~ (retiré le 2026-11-22 : D18 ; remplacé par `EX-002-16`)
- ~~**EX-002-08** (manuel) — `cargo test -p mcp-proxy-core secrets` : tous les tests passent ;
  `platform_store_round_trip` y utilise `WindowsMemoryProtector` et les tests du client HTTP passent par
  SChannel.~~ (retiré le 2026-11-22 : D18, `WindowsMemoryProtector` n'existe plus ; remplacé par `EX-002-17`)
- **EX-002-09** (manuel) — Aucun message « secrets en mémoire NON chiffrés » dans la sortie des tests : le
  protecteur de développement n'existe pas sous Windows.
- **EX-002-16** (manuel) — `cargo test -p mcp-proxy-core platform::windows` : les 2 tests passent (aller-retour DPAPI
  utilisateur réel par WinRT, données protégées différentes des données claires et sans la valeur en clair ; données
  quelconques refusées ou rendues vides).
- **EX-002-17** (manuel) — `cargo test -p mcp-proxy-core secrets` : tous les tests passent ;
  `platform_store_round_trip` y utilise `DpapiKeyWrapper` (aller-retour d'essai de `SecretStore::for_platform`
  compris) et les tests du client HTTP passent par SChannel.

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
- **2026-11-22** — Enveloppe sans `unsafe` (D18 : aucun `unsafe` dans le dépôt, demande de l'utilisateur).
  `CryptProtectMemory` n'est accessible qu'en `unsafe` : la protection mémoire devient une enveloppe. Une clé de
  session aléatoire de 256 bits scelle chaque valeur et chaque variante par XChaCha20-Poly1305 (nonce aléatoire,
  session en données associées) ; elle n'est gardée qu'enveloppée par le trait `KeyWrapper` (DPAPI utilisateur par
  WinRT sous Windows, enrobage de développement ailleurs) et déballée une fois par usage, puis effacée avant la
  closure. `SecretStore::new` et `SecretStore::for_platform` renvoient un `Result` et font un aller-retour d'essai ;
  échec fermé sans repli. Le trait `MemoryProtector`, `PROTECT_BLOCK_SIZE`, l'alignement sur 16 octets et
  `ProtectionFailure::Misaligned` disparaissent. Critères retirés : `EX-002-01`, `EX-002-07`, `EX-002-08` ; ajoutés :
  `EX-002-11` à `EX-002-17` (I002.4 à I002.9).
- **2026-11-22** — Spec 025 : le client authentifié injecte par un `SessionGate` (`StoreSessionGate` pour `new`,
  celui de la passerelle pour `through_gate`), peut être restreint à une seule origine, refuse `TRACE` et `CONNECT`,
  et marque refusé le secret envoyé sur un 401 ou un 403. Raison : les modules locaux doivent suivre l'échéance et
  l'authentification à la demande de leur session comme un MCP distant, et ne joindre que leur `base_url` (I025.2 à
  I025.5). Les critères de cette spec sont inchangés.
