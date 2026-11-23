// Every text of the main window, in French. Components never hold a text of their own.

/** French: 0 and 1 take the singular. */
function plural(count: number, one: string, many: string): string {
  return `${String(count)} ${count <= 1 ? one : many}`
}

/** `5 000`: thousands separated, as French writes them. */
function number(count: number): string {
  return count.toLocaleString('fr-FR')
}

/** Like `plural`, with the thousands separated. */
function many(count: number, one: string, several: string): string {
  return `${number(count)} ${count <= 1 ? one : several}`
}

/** « DEV, INT et REC », « PROD », « » */
export function list(items: readonly string[]): string {
  if (items.length <= 1) return items.join('')
  return `${items.slice(0, -1).join(', ')} et ${items[items.length - 1] ?? ''}`
}

export const fr = {
  app: {
    name: 'MCP Proxy',
    version: (version: string) => `version ${version}`,
    loading: 'Chargement…',
    unavailable: "MCP Proxy ne répond pas : l'état de la passerelle n'a pas pu être lu.",
    outsideApp:
      "Cette page s'affiche dans la fenêtre de MCP Proxy. Ouverte ailleurs, elle n'a accès à aucune donnée.",
    invalidState: "L'état reçu de MCP Proxy est illisible : rien n'est affiché.",
    update: 'Redémarrer pour mettre à jour',
    updateCritical: 'Mise à jour critique',
    updateHelp: (version: string) => `MCP Proxy ${version} est installé : il sera utilisé au prochain démarrage.`,
    updateCriticalHelp: (version: string) =>
      `MCP Proxy ${version} corrige un problème important : il redémarrera de lui-même dès que plus aucun appel n'est en cours.`,
    restart: {
      title: (version: string) => `Redémarrer pour passer à MCP Proxy ${version} ?`,
      sessions:
        'Les connexions sont fermées au redémarrage : il faudra vous reconnecter, et les appels en cours des clients LLM seront interrompus.',
      cancel: 'Annuler',
      confirm: 'Redémarrer',
    },
  },
  menu: {
    trigger: 'Menu',
    clients: 'Clients LLM…',
  },
  clients: {
    title: 'Clients LLM',
    intro:
      "MCP Proxy s'ajoute à la configuration des clients LLM sous le nom « mcp-proxy ». Seule cette entrée est modifiée, après un aperçu, et le fichier est copié avant chaque écriture.",
    loading: 'Lecture des configurations…',
    loadFailed: (reason: string) => `Configurations illisibles : ${reason}.`,
    command: 'Commande écrite dans chaque client :',
    notInstalled:
      "MCP Proxy n'est pas installé : il tourne depuis un emplacement provisoire. Les clients LLM sont toujours configurés avec l'exe installé, qui n'existe pas encore : installez d'abord MCP Proxy (relancez l'exe téléchargé et choisissez « Installer »). « Retirer » reste possible.",
    client: { claudeDesktop: 'Claude Desktop', vscode: 'VS Code' },
    variant: {
      'claude-desktop': 'installation classique',
      'claude-desktop-msix': 'paquet MSIX (Microsoft Store)',
      vscode: 'profil par défaut',
    },
    label: {
      'claude-desktop': 'Claude Desktop',
      'claude-desktop-msix': 'Claude Desktop (paquet MSIX)',
      vscode: 'VS Code',
    },
    state: {
      notInstalled: 'non installé',
      notConfigured: 'non configuré',
      configured: 'configuré',
      otherCommand: 'configuré avec un autre chemin',
      unreadable: 'fichier illisible',
    },
    otherCommand: (command: string | null) =>
      `L'entrée « mcp-proxy » lance ${command ?? 'une autre commande'}. « Installer » la remplace par l'exe installé, après aperçu.`,
    unreadable: (reason: string) =>
      `${reason}. MCP Proxy n'écrit rien dans ce fichier : « Installer » donne l'entrée à coller.`,
    otherServers: (count: number) =>
      count === 0 ? '' : `${plural(count, 'autre serveur', 'autres serveurs')} dans ce fichier.`,
    twoLocations:
      "Claude Desktop a deux fichiers de configuration sur ce poste. Installé par son paquet MSIX (Microsoft Store), il lit celui du paquet, et non celui qu'ouvre son bouton « Edit Config ». Configurez celui que lit votre installation, ou les deux, puis vérifiez dans Claude Desktop (Settings, Developer) que « mcp-proxy » y figure.",
    probablyIgnored: 'Sans doute pas lu : Claude Desktop installé par son paquet MSIX lit le fichier du paquet.',
    install: 'Installer',
    remove: 'Retirer',
    installIn: (client: string) => `Installer MCP Proxy dans ${client}`,
    removeFrom: (client: string) => `Retirer MCP Proxy de ${client}`,
    copy: 'Copier la configuration',
    copyHelp: "Pour tout autre client LLM : l'entrée « mcp-proxy » au format «\u00a0mcpServers\u00a0», le plus courant.",
    copied: 'Configuration copiée dans le presse-papiers :',
    copyFailed: 'Copie impossible : sélectionnez le texte ci-dessous et copiez-le (Ctrl+C).',
    close: 'Fermer',
    preview: {
      title: {
        install: (client: string) => `Ajouter MCP Proxy à ${client} ?`,
        remove: (client: string) => `Retirer MCP Proxy de ${client} ?`,
      },
      file: 'Fichier :',
      create: "Le fichier n'existe pas encore : il est créé avec cette seule entrée.",
      add: "L'entrée « mcp-proxy » est ajoutée.",
      replace: "L'entrée « mcp-proxy » actuelle est remplacée.",
      remove: "Seule l'entrée « mcp-proxy » est retirée.",
      untouched: (count: number) =>
        count === 0
          ? "Le reste du fichier n'est pas modifié."
          : `${count === 1 ? "L'autre serveur" : `Les ${String(count)} autres serveurs`} et le reste du fichier ne sont pas modifiés.`,
      otherFields: (keys: readonly string[]) => `autres champs : ${keys.join(', ')}`,
      backup: "Une copie du fichier actuel est enregistrée à côté de lui avant l'écriture.",
      nothing: {
        install: 'Déjà configuré : rien à écrire.',
        remove: "Aucune entrée « mcp-proxy » dans ce fichier : rien à retirer.",
      },
      manual: (reason: string) => `Ce fichier ne peut pas être modifié sans risque (${reason}) : MCP Proxy n'y écrit rien.`,
      manualInstall: 'Ajoutez vous-même cette configuration au fichier, puis redémarrez le client :',
      manualRemove: "Retirez vous-même l'entrée « mcp-proxy » de ce fichier, puis redémarrez le client.",
      changed:
        "Le fichier a changé depuis l'aperçu, sans doute réécrit par le client : rien n'a été écrit. Voici l'aperçu à jour.",
      cancel: 'Annuler',
      back: 'Retour',
      copy: "Copier l'entrée",
      working: 'Écriture…',
    },
    done: {
      install: (client: string) => `MCP Proxy a été ajouté à ${client}.`,
      remove: (client: string) => `MCP Proxy a été retiré de ${client}.`,
      nothing: 'Rien à écrire : le fichier était déjà comme demandé.',
      backup: (name: string) => `Copie de l'ancien fichier : ${name}, dans le même dossier.`,
      restart: {
        claudeDesktop:
          "Redémarrez Claude Desktop pour qu'il en tienne compte : quittez-le depuis son icône de la zone de notification (clic droit, « Quit »), et non en fermant seulement sa fenêtre, puis relancez-le.",
        vscode:
          "Redémarrez VS Code, ou rechargez sa fenêtre (commande « Developer: Reload Window »), pour qu'il en tienne compte.",
      },
      back: 'Retour à la liste',
    },
  },
  tabs: {
    label: 'Sections',
    connections: 'Connexions',
    mcp: 'MCP',
    journal: 'Journal',
  },
  profile: {
    label: 'Profil',
    trigger: (label: string) => `Profil : ${label}`,
    environments: (labels: readonly string[]) =>
      labels.length === 0 ? 'Aucun environnement exposé.' : `Environnements exposés : ${list(labels)}.`,
    sensitive: (labels: readonly string[]) => `Dont sensibles : ${list(labels)}.`,
    firstLaunch: {
      title: 'Choisissez votre profil',
      description:
        "Le profil décide des environnements exposés aux clients LLM et des règles appliquées à leurs appels. Vous pourrez en changer à tout moment, en haut de la fenêtre.",
      choices: 'Profils',
      confirm: 'Continuer',
    },
    change: {
      title: (label: string) => `Passer au profil « ${label} » ?`,
      exposed: (labels: readonly string[]) =>
        labels.length === 0
          ? 'Plus aucun environnement ne sera exposé aux clients LLM.'
          : `Les environnements exposés aux clients LLM reviennent aux valeurs par défaut de ce profil : ${list(labels)}.`,
      sensitive: (labels: readonly string[]) => `Environnements sensibles : ${list(labels)}.`,
      kept: 'Vos durcissements sont conservés.',
      cancel: 'Annuler',
      confirm: 'Changer de profil',
    },
  },
  banners: {
    suspended: "La passerelle est suspendue : tous les appels d'outils sont refusés.",
    resume: 'Reprendre',
    confirmations: (count: number) =>
      count === 1
        ? 'Une confirmation attend votre réponse.'
        : `${String(count)} confirmations attendent votre réponse.`,
    see: 'Voir',
    loginWaiting: (client: string, session: string) => `${client} attend une connexion à ${session}.`,
    unknownClient: 'Un client LLM',
    connect: 'Se connecter',
    commandFailed: (reason: string) => `Action impossible : ${reason}.`,
    dismiss: 'Fermer',
  },
  connections: {
    global: 'SSO globaux',
    perEnvironment: 'SSO par environnement',
    noEnvironment: "Aucun environnement n'est exposé aux clients LLM.",
    state: {
      neverConnected: 'jamais connectée',
      active: (until: string) => `active jusqu'à ${until}`,
      activeTomorrow: (until: string) => `active jusqu'à demain ${until}`,
      expired: 'expirée',
      loginRequested: 'connexion demandée',
    },
    services: 'Services',
    noService: 'Aucun service exposé',
    serviceIn: (mcp: string, environments: readonly string[]) => `${mcp} en ${environments.join(', ')}`,
    connect: 'Se connecter',
    disconnect: 'Déconnecter',
    connectTo: (session: string) => `Se connecter à ${session}`,
    disconnectFrom: (session: string) => `Déconnecter ${session}`,
  },
  mcp: {
    instanceName: (mcp: string, environment: string) => `${mcp} (${environment})`,
    remote: 'distant',
    local: 'local',
    noSession: 'aucune',
    category: { read: 'lecture', write: 'écriture', destructive: 'destructif' },
    categoryLetter: { read: 'L', write: 'É', destructive: 'D' },
    decision: {
      allow: 'autorisé',
      confirmElicitation: 'confirmation',
      confirmNotification: 'confirmation',
      deny: 'refusé',
    },
    decisionDetail: {
      allow: 'autorisé',
      confirmElicitation: 'confirmation dans le chat',
      confirmNotification: 'confirmation par notification',
      deny: 'refusé',
    },
    state: {
      catalogue: 'catalogue',
      connected: 'connecté',
      sessionRequired: 'session requise',
      unreachable: 'injoignable',
      disabled: 'coupé',
    },
    stateHelp: {
      catalogue:
        "Outils de l'instantané embarqué ou du cache : aucune liste n'a encore été lue auprès du serveur depuis le démarrage.",
      connected: 'La dernière lecture de la liste des outils a réussi.',
      sessionRequired: "Le serveur demande une session active : connectez-vous depuis l'onglet Connexions.",
      unreachable: 'La dernière lecture de la liste a échoué : la dernière liste connue est gardée.',
      disabled:
        "Coupé par vous : ses outils ne sont exposés dans aucun environnement et son serveur n'est pas contacté.",
    },
    environments: {
      heading: 'Environnements exposés',
      exposed: 'Exposé aux clients LLM.',
      notExposed: 'Non exposé : ses outils restent invisibles des clients LLM.',
      sensitive: 'Environnement sensible.',
      toggleOff: (label: string) => `Cliquez pour retirer ${label}.`,
      toggleOn: (label: string) => `Cliquez pour exposer ${label}.`,
      noProfile: "Choisissez d'abord un profil.",
      noneExposed:
        "Aucun environnement n'est exposé : les clients LLM ne voient aucun outil. Cliquez sur une pastille pour en exposer un.",
    },
    counter: {
      text: (count: number) => plural(count, 'outil exposé', 'outils exposés'),
      help: (threshold: number) =>
        `Au-delà de ${String(threshold)} outils exposés, ce compteur passe à l'orange : certains clients LLM plafonnent à 128 outils et ignorent les suivants.`,
      over: (count: number, threshold: number) =>
        `${String(count)} outils exposés, au-delà du seuil de ${String(threshold)} : certains clients LLM plafonnent à 128 outils et en ignorent une partie. Retirez un environnement ou coupez un MCP.`,
    },
    expose: {
      title: (label: string) => `Exposer ${label} aux clients LLM ?`,
      sensitive: (label: string) =>
        `${label} est un environnement sensible. Ses outils deviennent visibles des clients LLM.`,
      rules: (profile: string) =>
        `Les règles du profil « ${profile} » et les verrous de la configuration s'y appliquent ; la colonne Politique les montre.`,
      cancel: 'Annuler',
      confirm: 'Exposer',
    },
    update: {
      text: (name: string, added: number, modified: number, removed: number) => {
        const parts = [
          added > 0 ? plural(added, 'nouvel outil', 'nouveaux outils') : null,
          modified > 0 ? plural(modified, 'outil modifié', 'outils modifiés') : null,
          removed > 0 ? plural(removed, 'outil retiré', 'outils retirés') : null,
        ].filter((p): p is string => p !== null)
        return `${name} a été mis à jour : ${list(parts)}.`
      },
      see: 'Voir',
      seen: "J'ai vu",
      seeLabel: (name: string) => `Voir ${name}`,
      seenLabel: (name: string) => `J'ai vu la mise à jour de ${name}`,
    },
    search: {
      label: 'Rechercher',
      placeholder: 'Rechercher un MCP, un environnement, un outil…',
      clear: 'Effacer la recherche',
    },
    filter: {
      label: 'Environnement affiché',
      all: 'Tous',
      choice: (label: string, count: number) => `${label} · ${plural(count, 'MCP', 'MCP')}`,
      focus: (name: string) => `Filtré sur ${name}`,
      clearFocus: 'Retirer le filtre',
    },
    columns: {
      mcp: 'MCP',
      environment: 'Env.',
      session: 'Session',
      tools: 'Outils',
      policy: 'Politique',
      state: 'État',
      exposed: 'Exposé',
      sort: (column: string) => `Trier par ${column}`,
    },
    policy: {
      heading: 'Politique par défaut, pour un outil déclaré sans règle propre',
      line: (category: string, decision: string) => `${category} : ${decision}`,
    },
    row: {
      show: (name: string) => `Afficher les outils de ${name}`,
      hide: (name: string) => `Masquer les outils de ${name}`,
      expose: (mcp: string, environment: string) => `Exposer ${mcp} en ${environment}`,
      cutEverywhere: (mcp: string) => `Couper ${mcp} retire ses outils dans tous les environnements.`,
      restoreEverywhere: (mcp: string) => `Rétablir ${mcp} l'expose de nouveau dans les environnements exposés.`,
      reload: 'Recharger la liste',
      lastError: 'Dernière erreur',
    },
    blocked: {
      noProfile: "Choisissez d'abord un profil.",
      environment: (label: string) => `${label} n'est pas exposé : exposez-le d'abord avec sa pastille.`,
    },
    tools: {
      name: 'Outil',
      description: 'Description',
      category: 'Catégorie',
      policy: 'Politique',
      none: 'Aucun outil connu pour ce MCP.',
      noDescription: 'sans description',
      matching: (count: number, total: number) =>
        count === 1
          ? `1 outil sur ${String(total)} correspond à la recherche.`
          : `${String(count)} outils sur ${String(total)} correspondent à la recherche.`,
      removed: (names: readonly string[]) => `Retirés depuis votre dernière visite : ${names.join(', ')}.`,
      marks: { added: 'nouveau', unknown: 'inconnu', modified: 'modifié' },
      unknownHelp: 'Outil absent de la configuration : traité comme destructif.',
    },
    empty: {
      noResult: (query: string) => `Aucun MCP ne correspond à « ${query} ».`,
      noRow: 'Aucun MCP dans cet environnement.',
    },
  },
  journal: {
    kind: { call: 'appel', toolsChanged: "liste d'outils", panic: 'panic', event: 'événement' },
    slow: 'lent',
    slowHelp: 'Appel lent : son contenu est gardé pour comprendre ce qui a pris du temps.',
    columns: {
      time: 'Heure',
      client: 'Client',
      environment: 'Env.',
      mcp: 'MCP',
      tool: 'Outil',
      status: 'Statut',
      duration: 'Durée',
    },
    search: {
      label: 'Rechercher dans le journal',
      placeholder: 'Rechercher un client, un outil, un MCP, un statut…',
      clear: 'Effacer la recherche',
    },
    filter: {
      label: 'Environnement affiché',
      all: 'Tous',
      choice: (label: string, count: number) => `${label} · ${many(count, 'entrée', 'entrées')}`,
    },
    status: {
      label: 'Statut affiché',
      all: 'Tout',
      problems: 'Problèmes',
      policies: 'Politiques',
      problemsHelp: 'Erreur, bloqué, lent, fuite masquée, délai dépassé, connexion requise, limite de débit, annulé.',
      policiesHelp: 'Interdit, refusé, confirmation demandée, acceptée ou refusée, passerelle suspendue.',
    },
    live: {
      live: 'En direct',
      paused: 'En pause',
      pause: 'Mettre le journal en pause',
      resume: 'Reprendre le direct',
    },
    pending: (count: number) =>
      count === 0
        ? 'Aucune nouvelle entrée depuis la pause.'
        : `${many(count, 'nouvelle entrée', 'nouvelles entrées')} depuis la pause.`,
    showPending: 'Afficher',
    count: (shown: number, total: number) =>
      shown === total ? many(total, 'entrée', 'entrées') : `${number(shown)} sur ${many(total, 'entrée', 'entrées')}`,
    limit: (limit: number) => `La fenêtre garde les ${number(limit)} entrées les plus récentes.`,
    export: {
      button: 'Exporter un diagnostic',
      running: 'Export en cours…',
      saved: 'Diagnostic exporté :',
      // The core says « export de diagnostic impossible : accès refusé »: said once, as a sentence.
      failed: (reason: string) =>
        reason.startsWith('export de diagnostic impossible')
          ? `E${reason.slice(1)}.`
          : `Export de diagnostic impossible : ${reason}.`,
      dismiss: 'Fermer',
    },
    row: {
      show: (what: string) => `Afficher le détail : ${what}`,
      hide: (what: string) => `Masquer le détail : ${what}`,
    },
    details: {
      trace: 'Trace',
      client: 'Client',
      session: 'Session',
      noSession: 'aucune',
      policy: 'Politique',
      noPolicy: 'non résolue',
      masked: 'Masqué par le filtre',
      maskedCount: (count: number, kinds: readonly string[]) =>
        `${plural(count, 'valeur masquée', 'valeurs masquées')}${kinds.length > 0 ? ` (${kinds.join(', ')})` : ''}`,
      request: 'Requête, filtrée',
      response: 'Réponse, filtrée',
    },
    maskedKind: {
      knownSecret: 'secret connu',
      jwt: 'jeton JWT',
      authorizationHeader: 'en-tête Authorization',
      cookieHeader: 'cookie',
      sensitiveField: 'champ sensible',
      privateKey: 'clé privée',
      urlCredentials: "identifiants d'URL",
      other: 'autre',
    },
    empty: {
      none: 'Aucune entrée pour le moment. Les appels des clients LLM et les événements de MCP Proxy apparaîtront ici.',
      noResult: (query: string) => `Aucune entrée ne correspond à « ${query} ».`,
      noMatch: 'Aucune entrée ne correspond à ces filtres.',
      clear: 'Retirer les filtres',
    },
    loading: 'Lecture du journal…',
    unavailable: "Le journal n'a pas pu être lu : MCP Proxy ne l'a pas transmis.",
    invalid: "Le journal reçu de MCP Proxy est illisible : rien n'est affiché.",
  },
  basic: {
    attempt: (attempt: number, max: number) => `Essai ${String(attempt)} sur ${String(max)}`,
    login: 'Identifiant',
    password: 'Mot de passe',
    cancel: 'Annuler',
    submit: 'Se connecter',
  },
  confirm: {
    title: 'Confirmation demandée',
    tool: 'Outil',
    client: 'Demandé par',
    policy: 'Politique',
    arguments: 'Arguments',
    expired: "Délai dépassé : l'appel est refusé.",
    remaining: (left: string) => `Sans réponse, l'appel sera refusé dans ${left}.`,
    refuse: 'Refuser',
    approve: 'Autoriser une fois',
  },
  install: {
    install: {
      title: 'Installer MCP Proxy',
      intro: (version: string) =>
        `MCP Proxy ${version} s'installe pour votre compte Windows, sans droits d'administrateur.`,
      location: 'Dossier',
      shortcut: 'Un raccourci « MCP Proxy » est ajouté au menu Démarrer. Vos réglages et vos journaux ne bougent pas.',
      autostart: 'Démarrer MCP Proxy avec Windows',
      autostartHint: "À l'ouverture de votre session, MCP Proxy démarre sans fenêtre, dans la zone de notification.",
      running: 'Si MCP Proxy est déjà ouvert, il sera fermé puis relancé depuis son dossier.',
      later: 'Pas maintenant',
      confirm: 'Installer',
      working: 'Installation en cours…',
      failedTitle: "L'installation n'a pas abouti",
    },
    update: {
      title: 'Mettre à jour MCP Proxy',
      versions: (installed: string, version: string) =>
        `La version ${installed} est installée. Cet exe est la version ${version}.`,
      running:
        'MCP Proxy sera fermé puis relancé dans sa nouvelle version : les sessions ouvertes devront être reconnectées.',
      kept: 'Vos réglages, vos journaux et le démarrage avec Windows sont conservés.',
      later: (installed: string) => `Lancer la version ${installed}`,
      confirm: 'Mettre à jour',
      working: 'Mise à jour en cours…',
      failedTitle: "La mise à jour n'a pas abouti",
    },
    failed: {
      fallback: "MCP Proxy peut tout de même fonctionner depuis l'emplacement de cet exe, sans être installé.",
      quit: 'Quitter',
      continue: 'Continuer sans installer',
    },
  },
} as const
