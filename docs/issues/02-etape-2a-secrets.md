# [Étape 2a] core — store de secrets en mémoire

> Référence commune : `00-cadrage.md`. Dépend de : étape 1.

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

- `put(&self, session: &SessionId, credential: Credential)` : remplace le secret de la session, scelle la valeur et
  ses variantes de masquage (définies en 2b).
- `with_exposed<R>(&self, session, f: impl FnOnce(ExposedCredential<'_>) -> R) -> Result<R, SecretError>` :
  déchiffre dans un tampon `Zeroizing`, appelle `f`, efface. **Aucune API ne retourne la valeur en clair.**
- `with_variants<R>(&self, f: impl FnOnce(&[&[u8]]) -> R) -> R` : expose, le temps de la closure, les variantes de
  tous les secrets actifs, pour le filtre.
- `wipe(session)`, `wipe_all()`, `has(session)`.
- Un secret appartient à une session et une seule.

### Liaison secret ↔ destination

- `OriginGuard` : pour chaque session, l'ensemble des origines autorisées (schéma + hôte + port), dérivé de la
  config : URL des instances MCP qui utilisent la session, `base_url` des modules locaux, `probe_url`.
- `authorize(session, &Url) -> Result<(), SecretError>` : refuse toute autre origine et tout schéma non `https`
  (exception `localhost` pour les tests).

### Client HTTP authentifié

- `AuthenticatedHttpClient`, construit par le core pour une session. C'est ce que reçoivent les modules locaux.
- À chaque requête : `authorize`, puis injection des en-têtes au moment de l'envoi via `with_exposed`
  (`Authorization: Bearer …`, `Cookie: …`, `Authorization: Basic …`).
- L'appelant ne peut lire ni l'en-tête injecté ni le secret. Les en-têtes de réponse `Set-Cookie`, `Authorization`,
  `WWW-Authenticate`, `Proxy-Authenticate` sont retirés avant de rendre la réponse.
- Aucune redirection suivie (`redirect::Policy::none()`). TLS natif, vérification jamais désactivée.
- Délai par requête configurable.

## Tests

- Aller-retour `protect` / `unprotect`. Tests `#[cfg(windows)]` pour `CryptProtectMemory`, compilés en contrôle
  croisé.
- Aucune valeur sensible dans `Debug` ni `Display`, pour tous les types, avec une valeur sentinelle. Version
  property-based (proptest) : quelle que soit la valeur, elle n'apparaît dans aucune sortie.
- `put` remplace ; `wipe` vide la session ; `with_exposed` sur une session vide renvoie une erreur sans détail
  sensible.
- Après `Drop`, le tampon est à zéro (accès de test dédié).
- `authorize` : autre hôte, autre port, `http` refusés ; origine déclarée acceptée.
- `AuthenticatedHttpClient` contre un serveur local : en-tête présent côté serveur ; redirection vers une autre
  origine non suivie ; `Set-Cookie` absent de la réponse rendue ; requête vers une origine non déclarée refusée
  **avant** tout envoi.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Calcul des variantes de masquage et filtre (2b). Acquisition des secrets (étape 4).
