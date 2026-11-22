---
id: "003"
titre: "core — filtre de sortie"
etape: "2b"
statut: "prévue"
depend_de: ["002"]
suivi: "github#4"
---

# 003 — core — filtre de sortie

## Objectif

Garantir qu'aucun secret ne sort : ni vers le LLM, ni dans les logs, ni dans les confirmations affichées. C'est le
cœur de la sécurité, avec le moteur de politiques : les tests doivent être solides.

## À faire

### Liste noire des valeurs exactes

Pour chaque secret actif, calculer et sceller ces variantes au moment du `put` :

1. valeur brute ;
2. base64 standard avec les **3 décalages d'alignement** ;
3. base64url avec les 3 décalages ;
4. hexadécimal, minuscules et majuscules ;
5. encodage URL : pourcent-encodage (chiffres hexa majuscules et minuscules) et forme `x-www-form-urlencoded` ;
6. forme échappée JSON, quand elle diffère de la valeur brute ;
7. pour un JWT : la **signature seule** (3ᵉ segment), ainsi que ses octets décodés réencodés en base64 standard et
   en hexadécimal ;
8. pour le Basic : `base64(login:motdepasse)`, plus toutes les variantes du mot de passe ;
9. pour les cookies : chaque valeur, et sa forme décodée si elle est pourcent-encodée.

Décalages d'alignement — pour un secret `s` de `n` octets et un décalage `k ∈ {0, 1, 2}` : encoder `k` octets
quelconques suivis de `s`, puis ne garder que les caractères entièrement déterminés par `s`, c'est-à-dire retirer
les `ceil(8k/6)` premiers caractères et tout ce qui suit l'indice `floor(8(k+n)/6)`, remplissage `=` compris. On
retrouve ainsi le secret enfoui à n'importe quelle position d'un bloc encodé.

Règles de longueur : la valeur brute est toujours filtrée, quelle que soit sa longueur. Une variante dérivée de
moins de 8 caractères est ignorée, pour éviter de masquer n'importe quoi. Dédupliquer.

Recherche : pendant une passe, les variantes sont exposées par `SecretStore::with_variants`, cherchées avec
`memchr::memmem` sur des aiguilles empruntées, sans copie, puis effacées. Ne pas construire d'automate qui
conserverait les motifs en clair.

### Motifs génériques

- JWT : `eyJ…` en 3 segments base64url (et 5 segments pour un JWE).
- En-tête `Authorization` sous forme texte : `Authorization: Bearer|Basic|Negotiate|Digest|NTLM <valeur>`, et la
  forme `authorization=<valeur>`.
- Lignes `Set-Cookie:` et `Cookie:`.
- Champs sensibles. Dans un objet JSON : si le **dernier segment** du nom de clé (découpage camelCase, snake_case,
  kebab-case) est `password`, `passwd`, `pwd`, `secret`, `token`, `credential(s)`, `authorization` ou `cookie`, ou
  si la clé normalisée vaut `apikey`, `api_key`, `access_key`, `private_key`, `client_secret`, `session_id`,
  `x-api-key`, alors la valeur est masquée **si c'est une chaîne**. Nombres et booléens ne sont jamais touchés :
  `max_tokens: 4096` et `token_count: 12` restent intacts. Dans du texte : formes `clé=valeur` et `clé: valeur`
  pour les mêmes clés.
- Liste des clés sensibles surchargeable par `[filter]` dans la config, avec ces valeurs par défaut.
- Bonus si c'est simple : blocs `-----BEGIN … PRIVATE KEY-----` et mot de passe dans `schéma://user:motdepasse@hôte`.

### API

```rust
pub struct OutputFilter { /* motifs génériques compilés + accès au SecretStore */ }

impl OutputFilter {
    /// Parcourt clés et feuilles chaîne, applique liste noire puis motifs génériques.
    pub fn filter_json(&self, value: &mut serde_json::Value) -> FilterReport;
    pub fn filter_text<'a>(&self, text: &'a str) -> (std::borrow::Cow<'a, str>, FilterReport);
    /// Liste noire seule : sert à bloquer les arguments entrants.
    pub fn contains_known_secret(&self, value: &serde_json::Value) -> bool;
}

/// Ne contient jamais la valeur trouvée.
pub struct FilterReport { pub exact_hits: u32, pub generic_hits: u32, pub kinds: Vec<HitKind> }
```

- Marqueurs de remplacement stables : `[SECRET MASQUÉ]`, `[JWT MASQUÉ]`, `[EN-TÊTE MASQUÉ]`, `[CHAMP MASQUÉ]`.
  Un marqueur ne doit jamais redéclencher un motif.
- Ordre : liste noire, puis motifs génériques.
- Le filtre s'applique à tout ce qui sort : résultats d'outils (texte, `structuredContent`, ressources embarquées,
  blocs base64 d'image ou d'audio), listes d'outils (noms, descriptions, schémas), messages d'erreur, notifications
  relayées, lignes de log, export de diagnostic, arguments affichés dans une confirmation.
- Fournir un `MakeWriter` (ou une `Layer`) `tracing` qui passe chaque ligne formatée par `filter_text` avant
  écriture : défense en profondeur pour les logs.

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

## Vérification

`cargo xtask ci` vert.

## Hors périmètre

Branchement dans la passerelle (3c) et dans les logs (3e).
