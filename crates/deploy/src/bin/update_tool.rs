//! `update-tool`: the signing tools of the updater (spec 020), launched by `cargo xtask keygen | sign | verify |
//! check-release`. Exit codes: 0, 1 (failure), 2 (arguments).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mcp_proxy_deploy::update::release::{
    SIGNING_KEY_ENV, describe_key, generate, sign_exe, signing_key, verify_release,
};
use mcp_proxy_deploy::update::slots::{hex_key, parse_slot};
use mcp_proxy_deploy::update::{KeyRing, check_release, embedded_slots};
use mcp_proxy_deploy::version::Version;
use zeroize::Zeroizing;

const USAGE: &str = "\
usage:
  keygen --out <fichier>
      crée une paire de clés : la clé privée (64 caractères hexadécimaux) dans <fichier>, qui ne doit pas exister ;
      la clé publique s'affiche, à mettre dans crates/deploy/keys/current.pub (ou next.pub)
  sign --exe <exe> [--version X.Y.Z] [--critical true|false] [--key-env NOM] [--out <fichier>]
      écrit le manifeste signé <exe>.sig ; la clé privée est lue dans la variable NOM (ED25519_PRIVATE_KEY par
      défaut), jamais affichée
  verify --exe <exe> [--sig <fichier>] [--public-key <hex>]...
      vérifie le manifeste et l'exe avec les clés données, sinon avec les clés embarquées
  check-release [--exe <exe>]
      refuse la clé de développement et l'absence de clé courante ; avec --exe, vérifie aussi l'exe construit";

/// The public half of the development key, here only: the application never embeds it.
const DEV_PUB: &str = include_str!("../../keys/dev.pub");

struct Args {
    values: Vec<(String, String)>,
}

impl Args {
    fn parse(raw: &[String], known: &[&str]) -> Result<Self, String> {
        let mut values = Vec::new();
        let mut items = raw.iter();
        while let Some(name) = items.next() {
            if !known.contains(&name.as_str()) {
                return Err(format!("argument inconnu : {name}"));
            }
            let value = items.next().ok_or_else(|| format!("valeur manquante après {name}"))?;
            values.push((name.clone(), value.clone()));
        }
        Ok(Self { values })
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.values
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn required(&self, name: &str) -> Result<&str, String> {
        self.one(name).ok_or_else(|| format!("{name} attendu"))
    }
}

enum Failure {
    Usage(String),
    Failed(String),
}

fn read(path: &Path) -> Result<Vec<u8>, Failure> {
    std::fs::read(path).map_err(|e| Failure::Failed(format!("{} illisible : {}", path.display(), e.kind())))
}

fn keygen(args: &Args) -> Result<(), Failure> {
    let out = PathBuf::from(args.required("--out").map_err(Failure::Usage)?);
    let (seed, public) = generate().map_err(|_| Failure::Failed("générateur aléatoire indisponible".to_owned()))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&out)
        .map_err(|e| Failure::Failed(format!("{} impossible à créer : {}", out.display(), e.kind())))?;
    let line = Zeroizing::new(format!("{}\n", seed.as_str()));
    file.write_all(line.as_bytes())
        .map_err(|e| Failure::Failed(format!("{} impossible à écrire : {}", out.display(), e.kind())))?;
    println!(
        "clé privée écrite dans {} (à garder secrète, jamais dans le dépôt)",
        out.display()
    );
    println!("clé publique : {}", describe_key(&public));
    println!("à mettre dans crates/deploy/keys/current.pub (ou next.pub pour une rotation)");
    Ok(())
}

fn sign(args: &Args) -> Result<(), Failure> {
    let exe_path = PathBuf::from(args.required("--exe").map_err(Failure::Usage)?);
    let critical = match args.one("--critical").unwrap_or("false") {
        "true" => true,
        "false" => false,
        other => {
            return Err(Failure::Usage(format!(
                "--critical : true ou false attendu, pas « {other} »"
            )));
        }
    };
    let expected = match args.one("--version") {
        Some(text) => Some(
            Version::parse(text.trim_start_matches('v'))
                .map_err(|_| Failure::Usage(format!("--version : « {text} » n'est pas un semver")))?,
        ),
        None => None,
    };
    let variable = args.one("--key-env").unwrap_or(SIGNING_KEY_ENV);
    let key = signing_key(std::env::var(variable).ok().map(Zeroizing::new))
        .map_err(|e| Failure::Failed(format!("{variable} : {e}")))?;
    let exe = read(&exe_path)?;
    let manifest = sign_exe(&exe, expected.as_ref(), critical, &key).map_err(Failure::Failed)?;
    let out = args.one("--out").map_or_else(
        || {
            let mut name = exe_path.as_os_str().to_owned();
            name.push(".sig");
            PathBuf::from(name)
        },
        PathBuf::from,
    );
    std::fs::write(&out, manifest.render())
        .map_err(|e| Failure::Failed(format!("{} impossible à écrire : {}", out.display(), e.kind())))?;
    println!(
        "{} : version {}, sha256 {}, critical {}, key-id {}",
        out.display(),
        manifest.version,
        manifest.sha256_hex(),
        manifest.critical,
        manifest.key_id
    );
    Ok(())
}

fn verify(args: &Args) -> Result<(), Failure> {
    let exe_path = PathBuf::from(args.required("--exe").map_err(Failure::Usage)?);
    let sig_path = args.one("--sig").map_or_else(
        || {
            let mut name = exe_path.as_os_str().to_owned();
            name.push(".sig");
            PathBuf::from(name)
        },
        PathBuf::from,
    );
    let given = args.all("--public-key");
    let keys = if given.is_empty() {
        KeyRing::embedded()
    } else {
        let keys = given
            .iter()
            .map(|text| hex_key(text).ok_or_else(|| Failure::Usage(format!("--public-key illisible : {text}"))))
            .collect::<Result<Vec<_>, _>>()?;
        KeyRing::new(&keys).map_err(|e| Failure::Failed(e.to_string()))?
    };
    if keys.is_empty() {
        return Err(Failure::Failed(
            "aucune clé publique : ni --public-key, ni clé embarquée".to_owned(),
        ));
    }
    let manifest =
        verify_release(&read(&exe_path)?, &read(&sig_path)?, &keys).map_err(|e| Failure::Failed(e.to_string()))?;
    println!(
        "vérifié : version {}, critical {}, key-id {}",
        manifest.version, manifest.critical, manifest.key_id
    );
    Ok(())
}

fn check(args: &Args) -> Result<(), Failure> {
    let dev = parse_slot(DEV_PUB)
        .ok()
        .flatten()
        .ok_or_else(|| Failure::Failed("keys/dev.pub illisible".to_owned()))?;
    let exe = match args.one("--exe") {
        Some(path) => Some(read(Path::new(path))?),
        None => None,
    };
    let slots = embedded_slots();
    check_release(&slots, &dev, exe.as_deref()).map_err(Failure::Failed)?;
    for (name, slot) in ["courante", "suivante"].iter().zip(slots) {
        match slot {
            Some(key) => println!("clé {name} : {}", describe_key(&key)),
            None => println!("clé {name} : aucune"),
        }
    }
    println!("check-release : OK");
    Ok(())
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = raw.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let known: &[&str] = match command.as_str() {
        "keygen" => &["--out"],
        "sign" => &["--exe", "--version", "--critical", "--key-env", "--out"],
        "verify" => &["--exe", "--sig", "--public-key"],
        "check-release" => &["--exe"],
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = Args::parse(rest, known)
        .map_err(Failure::Usage)
        .and_then(|args| match command.as_str() {
            "keygen" => keygen(&args),
            "sign" => sign(&args),
            "verify" => verify(&args),
            _ => check(&args),
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("{command} : {message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Failed(message)) => {
            eprintln!("{command} : ÉCHEC : {message}");
            ExitCode::FAILURE
        }
    }
}
