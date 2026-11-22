---
id: "003"
titre: "core — filtre de sortie"
etape: "2b"
statut: "implémentée"
depend_de: ["002"]
suivi: "github#4"
---

# 003 — core — filtre de sortie

## Objectif

Garantir qu'aucun secret ne sort : ni vers le LLM, ni dans les logs, ni dans les confirmations affichées. C'est le
cœur de la sécurité, avec le moteur de politiques : les tests doivent être solides.

## À faire

### Liste noire des valeurs exactes

Pour chaque secret actif (et chaque secret retiré encore dans son délai de grâce, voir la spec 002), calculer et
sceller ces variantes au moment du `put` (`secrets::variants::derive_variants`) :

1. valeur brute ;
2. base64 standard : encodage complet, avec et sans remplissage, et les **3 décalages d'alignement** ;
3. base64url : de même ;
4. hexadécimal, minuscules et majuscules ;
5. encodage URL : pourcent-encodage RFC 3986 et forme `x-www-form-urlencoded`, chacun avec les chiffres hexa
   majuscules et minuscules ;
6. formes échappées JSON, quand elles diffèrent de la valeur brute : celle de `serde_json`, celle qui écrit `/` en
   `\/`, celle qui écrit tout caractère non ASCII en `\uxxxx` (Python) ;
7. pour toute valeur en forme de JWT (3 segments base64url, en-tête `eyJ…`) : la **signature seule** (3ᵉ segment),
   ainsi que ses octets décodés réencodés en base64 standard (avec et sans remplissage) et en hexadécimal (deux
   casses) ;
8. pour le Basic : `login:motdepasse` et `base64(login:motdepasse)` (avec et sans remplissage), plus toutes les
   variantes du mot de passe ;
9. pour les cookies : chaque valeur, et sa forme décodée si elle est pourcent-encodée ; la forme décodée est traitée
   comme une valeur brute (avec toutes ses variantes).

Décalages d'alignement — pour un secret `s` de `n` octets et un décalage `k ∈ {0, 1, 2}` : encoder `k` octets
quelconques suivis de `s`, puis ne garder que les caractères entièrement déterminés par `s`, c'est-à-dire retirer
les `ceil(8k/6)` premiers caractères et tout ce qui suit l'indice `floor(8(k+n)/6)`, remplissage `=` compris. On
retrouve ainsi le secret enfoui à n'importe quelle position d'un bloc encodé.

Règles de longueur : la valeur brute est toujours filtrée, quelle que soit sa longueur. Une variante dérivée de
moins de 8 caractères est ignorée, pour éviter de masquer n'importe quoi. Dédupliquer. Tous les tampons
intermédiaires sont `Zeroizing` et alloués à leur capacité finale (aucune réallocation ne laisse de copie).

Recherche : pendant une passe, les variantes sont exposées par `SecretStore::with_variants`, cherchées avec
`memchr::memmem` sur des aiguilles empruntées, sans copie, puis effacées. Ne pas construire d'automate qui
conserverait les motifs en clair.

Masquage : chaque occurrence est remplacée par `[SECRET MASQUÉ]` ; des occurrences qui se chevauchent ou se touchent
donnent un seul marqueur, une occurrence entièrement contenue dans un marqueur déjà présent est ignorée, une
occurrence qui chevauche un marqueur l'absorbe. Une occurrence qui couperait un caractère UTF-8 (forme décodée d'un
cookie) est élargie au caractère entier.

### Motifs génériques

Appliqués dans cet ordre, sur le texte déjà passé par la liste noire :

- Blocs `-----BEGIN … PRIVATE KEY-----` jusqu'au `-----END … PRIVATE KEY-----` (ou jusqu'à la fin du texte s'il
  manque) : remplacés en entier par `[SECRET MASQUÉ]`.
- Mot de passe dans `schéma://user:motdepasse@hôte` (jusqu'au dernier `@` de l'autorité) : `[SECRET MASQUÉ]`.
- JWT : `eyJ…` en 3 segments base64url (et 5 segments pour un JWE) : `[JWT MASQUÉ]`.
- Champs sensibles. Dans un objet JSON : si le **dernier segment** du nom de clé (découpage camelCase, snake_case,
  kebab-case, et entre lettres et chiffres ; un dernier segment purement numérique est ignoré, `password2` compte)
  est `password`, `passwd`, `pwd`, `secret`, `token`, `credential(s)`, `authorization` ou `cookie`, ou si la clé
  normalisée (minuscules, sans séparateur) vaut celle de `apikey`, `api_key`, `access_key`, `private_key`,
  `client_secret`, `session_id`, `x-api-key`, alors la valeur est masquée par `[CHAMP MASQUÉ]` **si c'est une
  chaîne**. Nombres et booléens ne sont jamais touchés par cette règle : `max_tokens: 4096` et `token_count: 12`
  restent intacts. Dans du texte, pour les mêmes clés :
  - clé entre guillemets (`"clé": "valeur"`, `'clé': 'valeur'`, et `\"clé\": \"valeur\"` pour du JSON sérialisé
    dans une chaîne) : même règle que le JSON, seule une valeur chaîne est masquée ;
  - clé nue, `clé=valeur` : la valeur va jusqu'au premier blanc, `&`, `;`, `,`, guillemet, `<` ou `>` ;
  - clé nue, `clé: valeur` : la valeur va jusqu'à la fin de la ligne ;
  - une valeur entre guillemets est masquée entre ses guillemets.
- En-tête `Authorization` sous forme texte : `Authorization: Bearer|Basic|Negotiate|Digest|NTLM <valeur>`, et la
  forme `authorization=<valeur>` (aussi `Proxy-Authorization`) : le schéma est conservé, la valeur devient
  `[EN-TÊTE MASQUÉ]`.
- Lignes `Set-Cookie:`, `Set-Cookie2:` et `Cookie:`, où qu'elles soient dans le texte : la valeur, jusqu'à la fin de
  la ligne, devient `[EN-TÊTE MASQUÉ]`.
- Liste des clés sensibles complétée par `[filter]` dans la config : `extra_key_segments` (derniers segments, lettres
  minuscules et chiffres ASCII) et `extra_keys` (noms complets, lettres, chiffres, `_`, `-`, `.`). Ces listes
  **s'ajoutent** aux valeurs par défaut, qui ne peuvent pas être retirées. Entrée vide, invalide ou en double :
  erreur de validation. La table est facultative.

### API

```rust
pub struct OutputFilter { /* motifs génériques compilés + accès au SecretStore */ }

impl OutputFilter {
    pub fn new(store: Arc<SecretStore>, config: &FilterConfig) -> Result<Self, FilterError>;
    /// Parcourt clés, feuilles chaîne et nombres, applique liste noire puis motifs génériques.
    /// En cas d'erreur, `value` est remplacé par `[SECRET MASQUÉ]`.
    pub fn filter_json(&self, value: &mut serde_json::Value) -> Result<FilterReport, FilterError>;
    pub fn filter_text<'a>(&self, text: &'a str)
        -> Result<(std::borrow::Cow<'a, str>, FilterReport), FilterError>;
    /// Liste noire seule (clés, chaînes, nombres) : sert à bloquer les arguments entrants.
    pub fn contains_known_secret(&self, value: &serde_json::Value) -> Result<bool, FilterError>;
}

/// Ne contient jamais la valeur trouvée, ni sa position. `kinds` est trié, sans doublon.
pub struct FilterReport { pub exact_hits: u32, pub generic_hits: u32, pub kinds: Vec<HitKind> }

pub enum HitKind { KnownSecret, Jwt, AuthorizationHeader, CookieHeader, SensitiveField, PrivateKey, UrlCredentials }

/// `Unavailable` : la liste noire n'a pas pu être lue. Aucune variante ne porte de valeur sensible.
pub enum FilterError { Unavailable(SecretError), Pattern }
```

- **Échec fermé.** Si `SecretStore::with_variants` échoue (une variante, active ou retirée, ne peut être
  déchiffrée), aucune comparaison n'a lieu : `filter_json`, `filter_text` et `contains_known_secret` renvoient
  `FilterError::Unavailable` et l'appelant ne transmet rien (pour un argument entrant : l'appel est bloqué).
  `filter_json` remplace en plus la valeur par `[SECRET MASQUÉ]`, pour qu'un appelant qui ignorerait l'erreur
  n'envoie rien.
- Un nombre JSON dont l'écriture décimale contient un secret connu devient une chaîne masquée.
- Deux clés d'un même objet qui ne différaient que par un secret et deviennent identiques après masquage ne sont pas
  renommées (le suffixe ajouté pourrait lui-même correspondre à un secret court) : leurs valeurs sont regroupées dans
  un tableau.

- Marqueurs de remplacement stables : `[SECRET MASQUÉ]`, `[JWT MASQUÉ]`, `[EN-TÊTE MASQUÉ]`, `[CHAMP MASQUÉ]`.
  Un marqueur ne doit jamais redéclencher un motif.
- Ordre : liste noire, puis motifs génériques. Les deux couches sont réappliquées jusqu'à ce que le texte ne change
  plus, ce qui rend le filtre idempotent ; au-delà de 8 passes, le texte entier est remplacé par `[SECRET MASQUÉ]`.
- Le filtre s'applique à tout ce qui sort : résultats d'outils (texte, `structuredContent`, ressources embarquées,
  blocs base64 d'image ou d'audio), listes d'outils (noms, descriptions, schémas), messages d'erreur, notifications
  relayées, lignes de log, export de diagnostic, arguments affichés dans une confirmation.
- Fournir un `MakeWriter` `tracing` (`FilteringMakeWriter`) qui passe chaque enregistrement formaté par
  `filter_text` avant écriture : défense en profondeur pour les logs. L'enregistrement est mis en tampon
  (`Zeroizing`) jusqu'à ce qu'il soit complet (écrivain relâché ou vidé) ; si le filtre est indisponible, une ligne
  fixe le remplace.

## Tests

Pour chaque variante, un test positif et un test de non-régression. En particulier :

- **EX-003-01** — secret enfoui aux décalages 0, 1 et 2 dans des octets aléatoires, puis encodé en base64 et en base64url :
  toujours détecté et masqué (test property-based) ;
- **EX-003-02** — hexadécimal dans les deux casses ; pourcent-encodage dans les deux casses ; forme JSON échappée ;
- **EX-003-03** — signature de JWT seule ; `base64(login:motdepasse)` ; valeur de cookie ;
- **EX-003-04** — secret dans une clé JSON, dans un tableau imbriqué, dans un `structuredContent` profond, au milieu d'un texte ;
- **EX-003-05** — secret court : valeur brute masquée, variantes dérivées courtes ignorées ;
- **EX-003-06** — motifs génériques : JWT quelconque, `Authorization` (Bearer et Basic), `Set-Cookie`, champs sensibles ;
  `max_tokens`, `token_count`, `tokenizer` intacts ;
- **EX-003-07** — idempotence : filtrer deux fois donne le même résultat ;
- **EX-003-08** — `contains_known_secret` : vrai pour chaque variante, faux pour un JWT inconnu ;
- **EX-003-09** — `FilterReport` ne contient jamais la valeur ;
- **EX-003-10** — le `MakeWriter` masque un secret émis par `tracing::info!` ;
- **EX-003-11** — propriété générale : pour tout secret et tout document JSON qui le contient sous une des variantes, la sortie ne
  contient plus aucune des variantes.
- **EX-003-12** — échec fermé : si la liste noire ne peut pas être lue, `filter_text`, `filter_json` et
  `contains_known_secret` renvoient une erreur sans valeur sensible, `filter_json` ne laisse que `[SECRET MASQUÉ]`
  dans le document, et le `MakeWriter` n'écrit que sa ligne fixe ;
- **EX-003-13** — un secret effacé (`wipe`) reste masqué pendant le délai de grâce du store, et ne l'est plus une fois
  le délai écoulé (horloge de test) ;
- **EX-003-14** — `[filter]` : les clés ajoutées sont masquées, les clés par défaut le restent ; table absente
  acceptée ; entrée invalide, en double ou clé inconnue refusée par la validation.

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Branchement dans la passerelle (3c) et dans les logs (3e), câblage du délai de grâce du store sur
`upstream_timeout_secs` + 10 s (étape 3).

## Amendements

- **2026-11-22** — Implémentation (I003.1 à I003.14). Écarts par rapport au texte initial, reportés ci-dessus :
  - **échec fermé** (décidé à la relecture de l'étape 2a) : `filter_json`, `filter_text` et
    `contains_known_secret` renvoient un `Result` ; en cas d'erreur, l'appelant ne transmet rien et `filter_json`
    remplace la valeur par un marqueur. `OutputFilter::new` renvoie aussi un `Result` (compilation des motifs, sans
    `unwrap`). Nouveau critère `EX-003-12` ;
  - **variantes retirées** (amendement de la spec 002 du même jour) : un secret effacé ou remplacé reste masqué
    pendant le délai de grâce. Nouveau critère `EX-003-13` ;
  - `[filter]` **complète** la liste des clés sensibles au lieu de la remplacer, pour qu'une config ne puisse pas
    affaiblir le filtre (INV-6). Nouveau critère `EX-003-14` ;
  - variantes ajoutées : encodages base64 complets avec et sans remplissage, deux formes JSON échappées de plus,
    `login:motdepasse` brut, signature JWT pour toute valeur en forme de JWT, forme décodée des cookies traitée
    comme brute ;
  - nombres JSON contenant un secret connu masqués ; clés en collision regroupées plutôt que renommées ;
  - règles précises des motifs génériques en texte (clés entre guillemets, valeur jusqu'à un délimiteur ou à la fin de
    la ligne, schéma `Authorization` conservé, lignes de cookies où qu'elles soient) ; les blocs de clé privée et les
    mots de passe d'URL utilisent `[SECRET MASQUÉ]` plutôt qu'un nouveau marqueur ;
  - point fixe : liste noire et motifs génériques réappliqués jusqu'à stabilité (8 passes au plus).
