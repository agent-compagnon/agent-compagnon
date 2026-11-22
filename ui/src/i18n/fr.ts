// Every text of the main window, in French. Components never hold a text of their own.

/** French: 0 and 1 take the singular. */
function plural(count: number, one: string, many: string): string {
  return `${String(count)} ${count <= 1 ? one : many}`
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
  },
  tabs: {
    label: 'Sections',
    connections: 'Connexions',
    mcp: 'MCP',
    journal: 'Journal',
  },
  shells: {
    journal: 'Les appels et les événements de la passerelle apparaîtront ici.',
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
} as const
