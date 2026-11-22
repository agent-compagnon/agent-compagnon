---
id: "029"
titre: "Exe Windows cross-compilé depuis Linux"
statut: "à la demande"
depend_de: []
suivi: "github#30"
---

# 029 — Exe Windows cross-compilé depuis Linux

> Décision D15 : à la demande, pas à chaque étape.

## Pourquoi

La recette se fait à la main sous Windows. Tant que le pipeline GitLab n'est pas en place, il faut un moyen
d'obtenir un `mcp-proxy.exe` à tester, sur demande.

## État des lieux

- Depuis le conteneur Linux de développement, `cargo check` et `cargo clippy` en cible
  `x86_64-pc-windows-msvc` fonctionnent, appli Tauri comprise : vérifié.
- **L'édition de liens ne fonctionne pas** : il manque la bibliothèque d'exécution MSVC et le SDK Windows.
  `clang` et `lld-link` sont déjà présents.

## À faire quand un exe est demandé

1. Installer `cargo-xwin`.
2. `cargo xwin build --release -p mcp-proxy --target x86_64-pc-windows-msvc`, après `npm ci` et `npm run build`
   dans `ui/`. cargo-xwin télécharge la bibliothèque d'exécution MSVC et le SDK Windows depuis les serveurs de
   Microsoft.
3. **Ces hôtes ne sont pas autorisés** dans l'environnement cloud actuel. Il faudra les ajouter à ce moment-là, sur
   la base du refus constaté, avec l'accord de l'utilisateur. L'usage du SDK est soumis à la licence Microsoft.
4. Envoyer l'exe produit dans la session, avec la liste des vérifications de `docs/recette.md` qui s'appliquent.
5. Consigner la commande exacte dans `README.md` et l'exposer en `cargo xtask build-win`.

## Limites à rappeler à chaque livraison

- L'exe n'est **pas signé** (ni ed25519 de release, ni Authenticode) : il embarque la clé publique de
  développement. À réserver aux essais.
- Il n'a été exécuté nulle part avant d'être livré.
