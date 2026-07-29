/**
 * PandaMUX i18n — 22 languages, browser detection, RTL support
 */
const LANGS = {
  en: "English",
  fr: "Français",
  es: "Español",
  de: "Deutsch",
  pt: "Português",
  it: "Italiano",
  nl: "Nederlands",
  pl: "Polski",
  tr: "Türkçe",
  ru: "Русский",
  uk: "Українська",
  ar: "العربية",
  zh: "简体中文",
  "zh-TW": "繁體中文",
  ja: "日本語",
  ko: "한국어",
  hi: "हिन्दी",
  vi: "Tiếng Việt",
  th: "ภาษาไทย",
  id: "Bahasa Indonesia",
  sv: "Svenska",
  cs: "Čeština",
};

const RTL_LANGS = ["ar"];

const T = {
  // ═══════════════════════════ ENGLISH ═══════════════════════════
  en: {
    "nav.docs": "Docs",
    "nav.changelog": "Changelog",
    "nav.community": "Community",
    "nav.github": "GitHub",
    "header.download": "Download for Windows",
    "hero.tagline": "The terminal built for",
    "hero.word1": "Claude Code",
    "hero.word2": "AI agents",
    "hero.word3": "multitasking",
    "hero.word4": "Windows",
    "hero.desc":
      "Native Windows application written in Rust. Vertical tabs, notifications when agents need attention, split panes, SSH remote sessions and a pipe API for automation.",
    "hero.download": "Download for Windows",
    "hero.github": "View on GitHub",
    "features.title": "Features",
    "f1.name": "Passive Claude Code integration",
    "f1.desc":
      "PandaMUX observes Claude Code without modifying its behavior. Auto-configured hooks report agent and tool activity to the sidebar. Zero configuration.",
    "f2.name": "Notification rings",
    "f2.desc":
      "panels glow blue when agents need attention. Windows toast notification, taskbar flash, and built-in notification center.",
    "f3.name": "Vertical tabs",
    "f3.desc":
      "the sidebar displays git branch, working directory, ports, agent count, PR status and notification text. Double-click to rename.",
    "f4.name": "Split panes",
    "f4.desc":
      "horizontal and vertical splits in each tab. Ctrl+D to split right, Ctrl+Shift+D to split down.",
    "f5.name": "Activity indicators",
    "f5.desc":
      "pulsing orange = working, green = done, red = interrupted. Visible at a glance in the sidebar.",
    "f6.name": "Saved sessions",
    "f6.desc":
      "save your splits and directories. Automatic restore on startup.",
    "f7.name": "Scriptable",
    "f7.desc":
      'CLI and named pipe API (<code>\\\\.\\pipe\\pandamux</code>) for automation and scripting. JSON-RPC v2 protocol compatible with cmux.',
    "f8.name": "Windows native",
    "f8.desc":
      "ConPTY for terminal emulation, Windows toast notifications, taskbar flash, native title bar overlay.",
    "f9.name": "GPU acceleration",
    "f9.desc": "powered by wgpu with GPU rendering for smooth display.",
    "f10.name": "Theme compatible",
    "f10.desc":
      "import your themes from Windows Terminal or Ghostty. 450+ bundled Ghostty themes.",
    "f11.name": "Keyboard shortcuts",
    "f11.desc":
      "full shortcuts for workspaces, splits, SSH sessions and more.",
    "faq.title": "Frequently asked questions",
    "faq1.q": "What is the relationship between PandaMUX and cmux?",
    "faq1.a":
      'PandaMUX is a Windows fork of <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux is a native macOS app (Swift + AppKit + Ghostty). PandaMUX reproduces the same experience on Windows using Rust + Iced + alacritty_terminal + ConPTY. The CLI/pipe protocol is compatible between the two.',
    "faq2.q": "What platforms are supported?",
    "faq2.a":
      'Windows 10 and 11 only (x64). For macOS, use <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>. For Linux, the project can theoretically be built from source but is not officially supported.',
    "faq3.q": "Which agents are compatible?",
    "faq3.a":
      "PandaMUX is optimized for Claude Code with automatic passive integration (hooks, CDP proxy, CLAUDE.md). Any command-line agent or program can be used in PandaMUX terminals — Codex, Gemini CLI, Aider, OpenCode, or any script.",
    "faq4.q": "How do notifications work?",
    "faq4.a":
      'Shell integration scripts detect when a command finishes or is interrupted. The pane gets a blue ring, the sidebar badge increments, and a Windows toast notification appears. Supports OSC 9/99/777, the <code>pandamux notify</code> CLI, and idle detection.',
    "faq5.q": "How is it different from Windows Terminal?",
    "faq5.a":
      "Windows Terminal is an excellent terminal emulator, but it has no notification system, no agent activity visibility, and no live metadata (git branch, ports, PR) in tabs. PandaMUX is specifically designed to supervise AI agents.",
    "faq6.q": "Is it free?",
    "faq6.a":
      'Yes. PandaMUX is open-source under the MIT license. Free download on <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Download for Windows",
    "cta.github": "View on GitHub",
    "footer.product": "Product",
    "footer.changelog": "Changelog",
    "footer.community": "Community",
    "footer.releases": "Releases",
    "footer.resources": "Resources",
    "footer.docs": "Documentation",
    "footer.install": "Installation",
    "footer.shortcuts": "Shortcuts",
    "footer.legal": "Legal",
    "footer.license": "MIT License",
    "footer.social": "Social",
    "footer.copyright": "© 2026 PandaMUX",
    "footer.fork": "Windows fork of",
    "plugin.badge": "Claude Code Plugin",
    "plugin.desc": "Launch multiple Claude Code agents in parallel from a single command. The plugin analyzes your codebase, splits the task into independent subtasks, and distributes them in waves with dependency management, isolated file zones and automatic review at the end of each cycle.",
    "plugin.install_label": "Install",
    "plugin.view_github": "View on GitHub",
    "plugin.docs": "Documentation",
  },

  // ═══════════════════════════ FRENCH ═══════════════════════════
  fr: {
    "nav.docs": "Docs",
    "nav.changelog": "Changelog",
    "nav.community": "Communauté",
    "nav.github": "GitHub",
    "header.download": "Télécharger pour Windows",
    "hero.tagline": "Le terminal conçu pour",
    "hero.word1": "Claude Code",
    "hero.word2": "les agents IA",
    "hero.word3": "le multitasking",
    "hero.word4": "Windows",
    "hero.desc":
      "Application Windows native écrite en Rust. Onglets verticaux, notifications quand les agents ont besoin d'attention, panneaux divisés, sessions SSH distantes et une API pipe pour l'automatisation.",
    "hero.download": "Télécharger pour Windows",
    "hero.github": "Voir sur GitHub",
    "features.title": "Fonctionnalités",
    "f1.name": "Intégration passive Claude Code",
    "f1.desc":
      "PandaMUX observe Claude Code sans modifier son comportement. Les hooks auto-configurés rapportent l'activité des agents et outils dans la barre latérale. Zéro configuration.",
    "f2.name": "Anneaux de notification",
    "f2.desc":
      "les panneaux s'illuminent en bleu quand les agents ont besoin d'attention. Notification Windows toast, flash barre des tâches, et centre de notifications intégré.",
    "f3.name": "Onglets verticaux",
    "f3.desc":
      "la barre latérale affiche la branche git, le répertoire de travail, les ports, le nombre d'agents, le statut PR et le texte de notification. Double-clic pour renommer.",
    "f4.name": "Panneaux divisés",
    "f4.desc":
      "divisions horizontales et verticales dans chaque onglet. Ctrl+D pour diviser à droite, Ctrl+Shift+D pour diviser en bas.",
    "f5.name": "Indicateurs d'activité",
    "f5.desc":
      "orange pulsant = en cours, vert = terminé, rouge = interrompu. Visible d'un coup d'œil dans la barre latérale.",
    "f6.name": "Sessions sauvegardées",
    "f6.desc":
      "sauvegardez vos splits et répertoires. Restauration automatique au démarrage.",
    "f7.name": "Scriptable",
    "f7.desc":
      'CLI et API named pipe (<code>\\\\.\\pipe\\pandamux</code>) pour l\'automatisation et le scripting. Protocole JSON-RPC v2 compatible avec cmux.',
    "f8.name": "Natif Windows",
    "f8.desc":
      "ConPTY pour l'émulation de terminal, notifications toast Windows, flash barre des tâches, title bar overlay native.",
    "f9.name": "Accélération GPU",
    "f9.desc":
      "propulsé par wgpu avec rendu GPU pour un affichage fluide.",
    "f10.name": "Thèmes compatibles",
    "f10.desc":
      "importez vos thèmes depuis Windows Terminal ou Ghostty. 450+ thèmes Ghostty inclus.",
    "f11.name": "Raccourcis clavier",
    "f11.desc":
      "raccourcis complets pour les espaces de travail, les divisions, les sessions SSH et plus.",
    "faq.title": "Questions fréquentes",
    "faq1.q": "Quelle est la relation entre PandaMUX et cmux ?",
    "faq1.a":
      'PandaMUX est un fork Windows de <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux est une application macOS native (Swift + AppKit + Ghostty). PandaMUX reproduit la même expérience sur Windows en utilisant Rust + Iced + alacritty_terminal + ConPTY. Le protocole CLI/pipe est compatible entre les deux.',
    "faq2.q": "Quelles plateformes sont supportées ?",
    "faq2.a":
      'Windows 10 et 11 uniquement (x64). Pour macOS, utilisez <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>. Pour Linux, le projet peut théoriquement être compilé depuis les sources mais ce n\'est pas officiellement supporté.',
    "faq3.q": "Quels agents sont compatibles ?",
    "faq3.a":
      "PandaMUX est optimisé pour Claude Code avec intégration passive automatique (hooks, CDP proxy, CLAUDE.md). Tout agent ou programme en ligne de commande peut être utilisé dans les terminaux PandaMUX — Codex, Gemini CLI, Aider, OpenCode, ou n'importe quel script.",
    "faq4.q": "Comment fonctionnent les notifications ?",
    "faq4.a":
      'Les scripts d\'intégration shell détectent quand une commande se termine ou est interrompue. Le panneau reçoit un anneau bleu, le badge sidebar s\'incrémente, et une notification Windows toast apparaît. Supporte OSC 9/99/777, la CLI <code>pandamux notify</code>, et la détection d\'inactivité.',
    "faq5.q": "En quoi c'est différent de Windows Terminal ?",
    "faq5.a":
      "Windows Terminal est un excellent émulateur de terminal, mais il n'a pas de système de notification, pas de visibilité sur l'activité des agents, et pas de métadonnées live (branche git, ports, PR) dans les onglets. PandaMUX est conçu spécifiquement pour superviser des agents IA.",
    "faq6.q": "C'est gratuit ?",
    "faq6.a":
      'Oui. PandaMUX est open-source sous licence MIT. Téléchargement gratuit sur <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Télécharger pour Windows",
    "cta.github": "Voir sur GitHub",
    "footer.product": "Produit",
    "footer.changelog": "Changelog",
    "footer.community": "Communauté",
    "footer.releases": "Releases",
    "footer.resources": "Ressources",
    "footer.docs": "Documentation",
    "footer.install": "Installation",
    "footer.shortcuts": "Raccourcis",
    "footer.legal": "Légal",
    "footer.license": "Licence MIT",
    "footer.social": "Social",
    "footer.copyright": "© 2026 PandaMUX",
    "footer.fork": "Fork Windows de",
    "plugin.badge": "Plugin Claude Code",
    "plugin.desc": "Lancez plusieurs agents Claude Code en parallèle depuis une seule commande. Le plugin analyse votre codebase, découpe la tâche en sous-tâches indépendantes, et les distribue en vagues avec gestion des dépendances, zones de fichiers isolées et revue automatique en fin de cycle.",
    "plugin.install_label": "Installation",
    "plugin.view_github": "Voir sur GitHub",
    "plugin.docs": "Documentation",
  },

  // ═══════════════════════════ SPANISH ═══════════════════════════
  es: {
    "nav.docs": "Docs",
    "nav.changelog": "Cambios",
    "nav.community": "Comunidad",
    "nav.github": "GitHub",
    "header.download": "Descargar para Windows",
    "hero.tagline": "La terminal diseñada para",
    "hero.word1": "Claude Code",
    "hero.word2": "agentes IA",
    "hero.word3": "multitarea",
    "hero.word4": "Windows",
    "hero.desc":
      "Aplicación Windows nativa escrita en Rust. Pestañas verticales, notificaciones cuando los agentes necesitan atención, paneles divididos, sesiones SSH remotas y una API pipe para automatización.",
    "hero.download": "Descargar para Windows",
    "hero.github": "Ver en GitHub",
    "features.title": "Características",
    "f1.name": "Integración pasiva con Claude Code",
    "f1.desc":
      "PandaMUX observa Claude Code sin modificar su comportamiento. Los hooks autoconfigurados reportan la actividad de agentes y herramientas en la barra lateral. Cero configuración.",
    "f2.name": "Anillos de notificación",
    "f2.desc":
      "los paneles se iluminan en azul cuando los agentes necesitan atención. Notificación toast de Windows, flash de barra de tareas y centro de notificaciones integrado.",
    "f3.name": "Pestañas verticales",
    "f3.desc":
      "la barra lateral muestra la rama git, el directorio de trabajo, los puertos, el número de agentes, el estado del PR y el texto de notificación. Doble clic para renombrar.",
    "f4.name": "Paneles divididos",
    "f4.desc":
      "divisiones horizontales y verticales en cada pestaña. Ctrl+D para dividir a la derecha, Ctrl+Shift+D para dividir abajo.",
    "f5.name": "Indicadores de actividad",
    "f5.desc":
      "naranja pulsante = trabajando, verde = terminado, rojo = interrumpido. Visible de un vistazo en la barra lateral.",
    "f6.name": "Sesiones guardadas",
    "f6.desc":
      "guarda tus divisiones y directorios. Restauración automática al inicio.",
    "f7.name": "Scriptable",
    "f7.desc":
      'CLI y API named pipe (<code>\\\\.\\pipe\\pandamux</code>) para automatización y scripting. Protocolo JSON-RPC v2 compatible con cmux.',
    "f8.name": "Nativo de Windows",
    "f8.desc":
      "ConPTY para emulación de terminal, notificaciones toast de Windows, flash de barra de tareas, title bar overlay nativo.",
    "f9.name": "Aceleración GPU",
    "f9.desc":
      "potenciado por wgpu con renderizado GPU para una visualización fluida.",
    "f10.name": "Temas compatibles",
    "f10.desc":
      "importa tus temas desde Windows Terminal o Ghostty. 450+ temas Ghostty incluidos.",
    "f11.name": "Atajos de teclado",
    "f11.desc":
      "atajos completos para espacios de trabajo, divisiones, sesiones SSH y más.",
    "faq.title": "Preguntas frecuentes",
    "faq1.q": "¿Cuál es la relación entre PandaMUX y cmux?",
    "faq1.a":
      'PandaMUX es un fork de Windows de <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux es una app nativa de macOS (Swift + AppKit + Ghostty). PandaMUX reproduce la misma experiencia en Windows usando Rust + Iced + alacritty_terminal + ConPTY. El protocolo CLI/pipe es compatible entre ambos.',
    "faq2.q": "¿Qué plataformas son compatibles?",
    "faq2.a":
      'Solo Windows 10 y 11 (x64). Para macOS, usa <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>. Para Linux, el proyecto puede compilarse desde el código fuente pero no es oficialmente soportado.',
    "faq3.q": "¿Qué agentes son compatibles?",
    "faq3.a":
      "PandaMUX está optimizado para Claude Code con integración pasiva automática (hooks, proxy CDP, CLAUDE.md). Cualquier agente o programa de línea de comandos puede usarse en los terminales PandaMUX — Codex, Gemini CLI, Aider, OpenCode o cualquier script.",
    "faq4.q": "¿Cómo funcionan las notificaciones?",
    "faq4.a":
      'Los scripts de integración shell detectan cuándo un comando termina o es interrumpido. El panel recibe un anillo azul, el badge de la barra lateral se incrementa y aparece una notificación toast de Windows. Soporta OSC 9/99/777, la CLI <code>pandamux notify</code> y detección de inactividad.',
    "faq5.q": "¿En qué se diferencia de Windows Terminal?",
    "faq5.a":
      "Windows Terminal es un excelente emulador de terminal, pero no tiene sistema de notificaciones, ni visibilidad de actividad de agentes, ni metadatos en vivo (rama git, puertos, PR) en las pestañas. PandaMUX está diseñado específicamente para supervisar agentes IA.",
    "faq6.q": "¿Es gratis?",
    "faq6.a":
      'Sí. PandaMUX es open-source bajo licencia MIT. Descarga gratuita en <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Descargar para Windows",
    "cta.github": "Ver en GitHub",
    "footer.product": "Producto",
    "footer.changelog": "Cambios",
    "footer.community": "Comunidad",
    "footer.releases": "Releases",
    "footer.resources": "Recursos",
    "footer.docs": "Documentación",
    "footer.install": "Instalación",
    "footer.shortcuts": "Atajos",
    "footer.legal": "Legal",
    "footer.license": "Licencia MIT",
    "footer.social": "Social",
    "footer.copyright": "© 2026 PandaMUX",
    "footer.fork": "Fork Windows de",
    "plugin.badge": "Plugin Claude Code",
    "plugin.desc": "Ejecuta múltiples agentes Claude Code en paralelo con un solo comando. El plugin analiza tu código, divide la tarea en subtareas independientes y las distribuye en oleadas con gestión de dependencias, zonas de archivos aisladas y revisión automática al final de cada ciclo.",
    "plugin.install_label": "Instalación",
    "plugin.view_github": "Ver en GitHub",
    "plugin.docs": "Documentación",
  },

  // ═══════════════════════════ GERMAN ═══════════════════════════
  de: {
    "nav.docs": "Docs",
    "nav.changelog": "Änderungen",
    "nav.community": "Community",
    "nav.github": "GitHub",
    "header.download": "Für Windows herunterladen",
    "hero.tagline": "Das Terminal für",
    "hero.word1": "Claude Code",
    "hero.word2": "KI-Agenten",
    "hero.word3": "Multitasking",
    "hero.word4": "Windows",
    "hero.desc":
      "Native Windows-Anwendung in Rust geschrieben. Vertikale Tabs, Benachrichtigungen wenn Agenten Aufmerksamkeit brauchen, geteilte Panels, SSH-Remote-Sitzungen und eine Pipe-API für Automatisierung.",
    "hero.download": "Für Windows herunterladen",
    "hero.github": "Auf GitHub ansehen",
    "features.title": "Funktionen",
    "f1.name": "Passive Claude Code Integration",
    "f1.desc":
      "PandaMUX beobachtet Claude Code ohne sein Verhalten zu ändern. Automatisch konfigurierte Hooks melden Agenten- und Tool-Aktivität in der Seitenleiste. Null Konfiguration.",
    "f2.name": "Benachrichtigungsringe",
    "f2.desc":
      "Panels leuchten blau auf wenn Agenten Aufmerksamkeit brauchen. Windows Toast-Benachrichtigung, Taskleisten-Flash und integriertes Benachrichtigungscenter.",
    "f3.name": "Vertikale Tabs",
    "f3.desc":
      "die Seitenleiste zeigt Git-Branch, Arbeitsverzeichnis, Ports, Agenten-Anzahl, PR-Status und Benachrichtigungstext. Doppelklick zum Umbenennen.",
    "f4.name": "Geteilte Panels",
    "f4.desc":
      "horizontale und vertikale Teilungen in jedem Tab. Strg+D zum Rechts-Teilen, Strg+Umschalt+D zum Unten-Teilen.",
    "f5.name": "Aktivitätsindikatoren",
    "f5.desc":
      "pulsierendes Orange = arbeitet, Grün = fertig, Rot = unterbrochen. Auf einen Blick in der Seitenleiste sichtbar.",
    "f6.name": "Gespeicherte Sitzungen",
    "f6.desc":
      "speichere deine Splits und Verzeichnisse. Automatische Wiederherstellung beim Start.",
    "f7.name": "Skriptbar",
    "f7.desc":
      'CLI und Named-Pipe-API (<code>\\\\.\\pipe\\pandamux</code>) für Automatisierung und Skripting. JSON-RPC v2 Protokoll kompatibel mit cmux.',
    "f8.name": "Windows-nativ",
    "f8.desc":
      "ConPTY für Terminal-Emulation, Windows Toast-Benachrichtigungen, Taskleisten-Flash, native Titelleisten-Overlay.",
    "f9.name": "GPU-Beschleunigung",
    "f9.desc":
      "angetrieben von wgpu mit GPU-Rendering für flüssige Darstellung.",
    "f10.name": "Theme-kompatibel",
    "f10.desc":
      "importiere deine Themes aus Windows Terminal oder Ghostty. 450+ mitgelieferte Ghostty-Themes.",
    "f11.name": "Tastenkürzel",
    "f11.desc":
      "vollständige Tastenkürzel für Arbeitsbereiche, Splits, SSH-Sitzungen und mehr.",
    "faq.title": "Häufig gestellte Fragen",
    "faq1.q": "Was ist die Beziehung zwischen PandaMUX und cmux?",
    "faq1.a":
      'PandaMUX ist ein Windows-Fork von <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux ist eine native macOS-App (Swift + AppKit + Ghostty). PandaMUX reproduziert die gleiche Erfahrung unter Windows mit Rust + Iced + alacritty_terminal + ConPTY. Das CLI/Pipe-Protokoll ist zwischen beiden kompatibel.',
    "faq2.q": "Welche Plattformen werden unterstützt?",
    "faq2.a":
      'Nur Windows 10 und 11 (x64). Für macOS verwende <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>. Für Linux kann das Projekt theoretisch aus dem Quellcode gebaut werden, wird aber nicht offiziell unterstützt.',
    "faq3.q": "Welche Agenten sind kompatibel?",
    "faq3.a":
      "PandaMUX ist für Claude Code mit automatischer passiver Integration optimiert (Hooks, CDP-Proxy, CLAUDE.md). Jeder Kommandozeilen-Agent oder jedes Programm kann in PandaMUX-Terminals verwendet werden — Codex, Gemini CLI, Aider, OpenCode oder jedes Skript.",
    "faq4.q": "Wie funktionieren die Benachrichtigungen?",
    "faq4.a":
      'Shell-Integrationsskripte erkennen wenn ein Befehl endet oder unterbrochen wird. Das Panel bekommt einen blauen Ring, das Sidebar-Badge erhöht sich und eine Windows Toast-Benachrichtigung erscheint. Unterstützt OSC 9/99/777, die CLI <code>pandamux notify</code> und Leerlauferkennung.',
    "faq5.q": "Wie unterscheidet es sich von Windows Terminal?",
    "faq5.a":
      "Windows Terminal ist ein hervorragender Terminal-Emulator, hat aber kein Benachrichtigungssystem, keine Agenten-Aktivitätssichtbarkeit und keine Live-Metadaten (Git-Branch, Ports, PR) in den Tabs. PandaMUX ist speziell für die Überwachung von KI-Agenten konzipiert.",
    "faq6.q": "Ist es kostenlos?",
    "faq6.a":
      'Ja. PandaMUX ist Open-Source unter der MIT-Lizenz. Kostenloser Download auf <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Für Windows herunterladen",
    "cta.github": "Auf GitHub ansehen",
    "footer.product": "Produkt",
    "footer.changelog": "Änderungen",
    "footer.community": "Community",
    "footer.releases": "Releases",
    "footer.resources": "Ressourcen",
    "footer.docs": "Dokumentation",
    "footer.install": "Installation",
    "footer.shortcuts": "Tastenkürzel",
    "footer.legal": "Rechtliches",
    "footer.license": "MIT-Lizenz",
    "footer.social": "Social",
    "footer.copyright": "© 2026 PandaMUX",
    "footer.fork": "Windows-Fork von",
    "plugin.badge": "Claude Code Plugin",
    "plugin.desc": "Starten Sie mehrere Claude Code-Agenten parallel mit einem einzigen Befehl. Das Plugin analysiert Ihre Codebasis, teilt die Aufgabe in unabhängige Teilaufgaben auf und verteilt sie in Wellen mit Abhängigkeitsmanagement, isolierten Dateizonen und automatischer Überprüfung am Ende jedes Zyklus.",
    "plugin.install_label": "Installation",
    "plugin.view_github": "Auf GitHub ansehen",
    "plugin.docs": "Dokumentation",
  },

  // ═══════════════════════════ PORTUGUESE ═══════════════════════════
  pt: {
    "nav.docs": "Docs",
    "nav.changelog": "Changelog",
    "nav.community": "Comunidade",
    "nav.github": "GitHub",
    "header.download": "Baixar para Windows",
    "hero.tagline": "O terminal feito para",
    "hero.word1": "Claude Code",
    "hero.word2": "agentes IA",
    "hero.word3": "multitarefa",
    "hero.word4": "Windows",
    "hero.desc":
      "Aplicação Windows nativa escrita em Rust. Abas verticais, notificações quando os agentes precisam de atenção, painéis divididos, sessões SSH remotas e uma API pipe para automação.",
    "hero.download": "Baixar para Windows",
    "hero.github": "Ver no GitHub",
    "features.title": "Funcionalidades",
    "f1.name": "Integração passiva com Claude Code",
    "f1.desc": "PandaMUX observa o Claude Code sem modificar seu comportamento. Hooks autoconfigurados reportam atividade de agentes e ferramentas na barra lateral. Zero configuração.",
    "f2.name": "Anéis de notificação",
    "f2.desc": "os painéis brilham em azul quando os agentes precisam de atenção. Notificação toast do Windows, flash da barra de tarefas e centro de notificações integrado.",
    "f3.name": "Abas verticais",
    "f3.desc": "a barra lateral mostra branch git, diretório de trabalho, portas, contagem de agentes, status do PR e texto de notificação. Duplo clique para renomear.",
    "f4.name": "Painéis divididos",
    "f4.desc": "divisões horizontais e verticais em cada aba. Ctrl+D para dividir à direita, Ctrl+Shift+D para dividir abaixo.",
    "f5.name": "Indicadores de atividade",
    "f5.desc": "laranja pulsante = trabalhando, verde = concluído, vermelho = interrompido. Visível rapidamente na barra lateral.",
    "f6.name": "Sessões salvas",
    "f6.desc": "salve suas divisões e diretórios. Restauração automática na inicialização.",
    "f7.name": "Scriptável",
    "f7.desc": 'CLI e API named pipe (<code>\\\\.\\pipe\\pandamux</code>) para automação e scripting. Protocolo JSON-RPC v2 compatível com cmux.',
    "f8.name": "Nativo Windows",
    "f8.desc": "ConPTY para emulação de terminal, notificações toast do Windows, flash da barra de tarefas, title bar overlay nativo.",
    "f9.name": "Aceleração GPU",
    "f9.desc": "alimentado por wgpu com renderização GPU para exibição fluida.",
    "f10.name": "Temas compatíveis",
    "f10.desc": "importe seus temas do Windows Terminal ou Ghostty. 450+ temas Ghostty incluídos.",
    "f11.name": "Atalhos de teclado",
    "f11.desc": "atalhos completos para workspaces, divisões, sessões SSH e mais.",
    "faq.title": "Perguntas frequentes",
    "faq1.q": "Qual é a relação entre PandaMUX e cmux?",
    "faq1.a": 'PandaMUX é um fork Windows de <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux é um app nativo macOS (Swift + AppKit + Ghostty). PandaMUX reproduz a mesma experiência no Windows usando Rust + Iced + alacritty_terminal + ConPTY. O protocolo CLI/pipe é compatível entre os dois.',
    "faq2.q": "Quais plataformas são suportadas?",
    "faq2.a": 'Apenas Windows 10 e 11 (x64). Para macOS, use <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>. Para Linux, o projeto pode ser compilado a partir do código-fonte mas não é oficialmente suportado.',
    "faq3.q": "Quais agentes são compatíveis?",
    "faq3.a": "PandaMUX é otimizado para Claude Code com integração passiva automática (hooks, proxy CDP, CLAUDE.md). Qualquer agente ou programa de linha de comando pode ser usado nos terminais PandaMUX — Codex, Gemini CLI, Aider, OpenCode ou qualquer script.",
    "faq4.q": "Como funcionam as notificações?",
    "faq4.a": 'Scripts de integração shell detectam quando um comando termina ou é interrompido. O painel recebe um anel azul, o badge da sidebar incrementa e uma notificação toast do Windows aparece. Suporta OSC 9/99/777, a CLI <code>pandamux notify</code> e detecção de inatividade.',
    "faq5.q": "Qual a diferença para o Windows Terminal?",
    "faq5.a": "Windows Terminal é um excelente emulador de terminal, mas não tem sistema de notificações, visibilidade de atividade de agentes ou metadados em tempo real (branch git, portas, PR) nas abas. PandaMUX é projetado especificamente para supervisionar agentes IA.",
    "faq6.q": "É grátis?",
    "faq6.a": 'Sim. PandaMUX é open-source sob licença MIT. Download gratuito em <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Baixar para Windows",
    "cta.github": "Ver no GitHub",
    "footer.product": "Produto",
    "footer.changelog": "Changelog",
    "footer.community": "Comunidade",
    "footer.releases": "Releases",
    "footer.resources": "Recursos",
    "footer.docs": "Documentação",
    "footer.install": "Instalação",
    "footer.shortcuts": "Atalhos",
    "footer.legal": "Legal",
    "footer.license": "Licença MIT",
    "footer.social": "Social",
    "footer.copyright": "© 2026 PandaMUX",
    "footer.fork": "Fork Windows de",
    "plugin.badge": "Plugin Claude Code",
    "plugin.desc": "Execute vários agentes Claude Code em paralelo com um único comando. O plugin analisa sua base de código, divide a tarefa em subtarefas independentes e as distribui em ondas com gerenciamento de dependências, zonas de arquivo isoladas e revisão automática ao final de cada ciclo.",
    "plugin.install_label": "Instalação",
    "plugin.view_github": "Ver no GitHub",
    "plugin.docs": "Documentação",
  },

  // ═══════════════════════════ ITALIAN ═══════════════════════════
  it: {
    "nav.docs": "Docs",
    "nav.changelog": "Changelog",
    "nav.community": "Comunità",
    "nav.github": "GitHub",
    "header.download": "Scarica per Windows",
    "hero.tagline": "Il terminale progettato per",
    "hero.word1": "Claude Code",
    "hero.word2": "agenti IA",
    "hero.word3": "il multitasking",
    "hero.word4": "Windows",
    "hero.desc": "Applicazione Windows nativa scritta in Rust. Tab verticali, notifiche quando gli agenti necessitano attenzione, pannelli divisi, sessioni SSH remote e un'API pipe per l'automazione.",
    "hero.download": "Scarica per Windows",
    "hero.github": "Vedi su GitHub",
    "features.title": "Funzionalità",
    "f1.name": "Integrazione passiva Claude Code",
    "f1.desc": "PandaMUX osserva Claude Code senza modificare il suo comportamento. Gli hook autoconfigurati riportano l'attività di agenti e strumenti nella barra laterale. Zero configurazione.",
    "f2.name": "Anelli di notifica",
    "f2.desc": "i pannelli si illuminano di blu quando gli agenti necessitano attenzione. Notifica toast Windows, flash barra delle applicazioni e centro notifiche integrato.",
    "f3.name": "Tab verticali",
    "f3.desc": "la barra laterale mostra il branch git, la directory di lavoro, le porte, il numero di agenti, lo stato PR e il testo di notifica. Doppio clic per rinominare.",
    "f4.name": "Pannelli divisi",
    "f4.desc": "divisioni orizzontali e verticali in ogni tab. Ctrl+D per dividere a destra, Ctrl+Shift+D per dividere in basso.",
    "f5.name": "Indicatori di attività",
    "f5.desc": "arancione pulsante = in corso, verde = completato, rosso = interrotto. Visibile a colpo d'occhio nella barra laterale.",
    "f6.name": "Sessioni salvate",
    "f6.desc": "salva le tue divisioni e directory. Ripristino automatico all'avvio.",
    "f7.name": "Scriptabile",
    "f7.desc": 'CLI e API named pipe (<code>\\\\.\\pipe\\pandamux</code>) per automazione e scripting. Protocollo JSON-RPC v2 compatibile con cmux.',
    "f8.name": "Nativo Windows",
    "f8.desc": "ConPTY per l'emulazione del terminale, notifiche toast Windows, flash barra delle applicazioni, title bar overlay nativo.",
    "f9.name": "Accelerazione GPU",
    "f9.desc": "alimentato da wgpu con rendering GPU per una visualizzazione fluida.",
    "f10.name": "Temi compatibili",
    "f10.desc": "importa i tuoi temi da Windows Terminal o Ghostty. 450+ temi Ghostty inclusi.",
    "f11.name": "Scorciatoie da tastiera",
    "f11.desc": "scorciatoie complete per workspace, divisioni, sessioni SSH e altro.",
    "faq.title": "Domande frequenti",
    "faq1.q": "Qual è la relazione tra PandaMUX e cmux?",
    "faq1.a": 'PandaMUX è un fork Windows di <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux è un\'app nativa macOS (Swift + AppKit + Ghostty). PandaMUX riproduce la stessa esperienza su Windows usando Rust + Iced + alacritty_terminal + ConPTY. Il protocollo CLI/pipe è compatibile tra i due.',
    "faq2.q": "Quali piattaforme sono supportate?",
    "faq2.a": 'Solo Windows 10 e 11 (x64). Per macOS, usa <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>. Per Linux, il progetto può essere compilato dai sorgenti ma non è ufficialmente supportato.',
    "faq3.q": "Quali agenti sono compatibili?",
    "faq3.a": "PandaMUX è ottimizzato per Claude Code con integrazione passiva automatica (hook, proxy CDP, CLAUDE.md). Qualsiasi agente o programma da riga di comando può essere usato nei terminali PandaMUX — Codex, Gemini CLI, Aider, OpenCode o qualsiasi script.",
    "faq4.q": "Come funzionano le notifiche?",
    "faq4.a": 'Gli script di integrazione shell rilevano quando un comando finisce o viene interrotto. Il pannello riceve un anello blu, il badge della sidebar si incrementa e appare una notifica toast Windows. Supporta OSC 9/99/777, la CLI <code>pandamux notify</code> e il rilevamento di inattività.',
    "faq5.q": "In cosa si differenzia da Windows Terminal?",
    "faq5.a": "Windows Terminal è un eccellente emulatore di terminale, ma non ha un sistema di notifiche, visibilità sull'attività degli agenti o metadati live (branch git, porte, PR) nei tab. PandaMUX è progettato specificamente per supervisionare agenti IA.",
    "faq6.q": "È gratuito?",
    "faq6.a": 'Sì. PandaMUX è open-source sotto licenza MIT. Download gratuito su <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Scarica per Windows",
    "cta.github": "Vedi su GitHub",
    "footer.product": "Prodotto",
    "footer.changelog": "Changelog",
    "footer.community": "Comunità",
    "footer.releases": "Releases",
    "footer.resources": "Risorse",
    "footer.docs": "Documentazione",
    "footer.install": "Installazione",
    "footer.shortcuts": "Scorciatoie",
    "footer.legal": "Legale",
    "footer.license": "Licenza MIT",
    "footer.social": "Social",
    "footer.copyright": "© 2026 PandaMUX",
    "footer.fork": "Fork Windows di",
    "plugin.badge": "Plugin Claude Code",
    "plugin.desc": "Avvia più agenti Claude Code in parallelo con un singolo comando. Il plugin analizza il tuo codice, suddivide il compito in sotto-attività indipendenti e le distribuisce a ondate con gestione delle dipendenze, zone di file isolate e revisione automatica alla fine di ogni ciclo.",
    "plugin.install_label": "Installazione",
    "plugin.view_github": "Vedi su GitHub",
    "plugin.docs": "Documentazione",
  },

  // ═══════════════════════════ DUTCH ═══════════════════════════
  nl: {
    "nav.docs": "Docs", "nav.changelog": "Changelog", "nav.community": "Community", "nav.github": "GitHub",
    "header.download": "Downloaden voor Windows",
    "hero.tagline": "De terminal gebouwd voor", "hero.word1": "Claude Code", "hero.word2": "AI-agenten", "hero.word3": "multitasking", "hero.word4": "Windows",
    "hero.desc": "Native Windows-applicatie geschreven in Rust. Verticale tabbladen, meldingen wanneer agenten aandacht nodig hebben, gesplitste panelen, SSH-sessies op afstand en een pipe-API voor automatisering.",
    "hero.download": "Downloaden voor Windows", "hero.github": "Bekijk op GitHub",
    "features.title": "Functies",
    "f1.name": "Passieve Claude Code integratie", "f1.desc": "PandaMUX observeert Claude Code zonder het gedrag te wijzigen. Automatisch geconfigureerde hooks rapporteren agent- en tool-activiteit in de zijbalk. Nul configuratie.",
    "f2.name": "Notificatieringen", "f2.desc": "panelen lichten blauw op wanneer agenten aandacht nodig hebben. Windows toast-melding, taakbalk-flash en ingebouwd meldingscentrum.",
    "f3.name": "Verticale tabbladen", "f3.desc": "de zijbalk toont git-branch, werkmap, poorten, aantal agenten, PR-status en meldingstekst. Dubbelklik om te hernoemen.",
    "f4.name": "Gesplitste panelen", "f4.desc": "horizontale en verticale splitsingen in elk tabblad. Ctrl+D om rechts te splitsen, Ctrl+Shift+D om onder te splitsen.",
    "f5.name": "Activiteitsindicatoren", "f5.desc": "pulserend oranje = bezig, groen = klaar, rood = onderbroken. In één oogopslag zichtbaar in de zijbalk.",
    "f6.name": "Opgeslagen sessies", "f6.desc": "sla je splitsingen en mappen op. Automatisch herstel bij opstarten.",
    "f7.name": "Scriptbaar", "f7.desc": 'CLI en named pipe API (<code>\\\\.\\pipe\\pandamux</code>) voor automatisering en scripting. JSON-RPC v2 protocol compatibel met cmux.',
    "f8.name": "Windows-natief", "f8.desc": "ConPTY voor terminal-emulatie, Windows toast-meldingen, taakbalk-flash, native titelbalk-overlay.",
    "f9.name": "GPU-versnelling", "f9.desc": "aangedreven door wgpu met GPU-rendering voor vloeiende weergave.",
    "f10.name": "Thema-compatibel", "f10.desc": "importeer je thema's uit Windows Terminal of Ghostty. 450+ meegeleverde Ghostty-thema's.",
    "f11.name": "Sneltoetsen", "f11.desc": "volledige sneltoetsen voor werkruimtes, splitsingen, SSH-sessies en meer.",
    "faq.title": "Veelgestelde vragen",
    "faq1.q": "Wat is de relatie tussen PandaMUX en cmux?", "faq1.a": 'PandaMUX is een Windows-fork van <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux is een native macOS-app (Swift + AppKit + Ghostty). PandaMUX reproduceert dezelfde ervaring op Windows met Rust + Iced + alacritty_terminal + ConPTY.',
    "faq2.q": "Welke platformen worden ondersteund?", "faq2.a": 'Alleen Windows 10 en 11 (x64). Voor macOS gebruik <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Welke agenten zijn compatibel?", "faq3.a": "PandaMUX is geoptimaliseerd voor Claude Code. Elke commandoregel-agent kan worden gebruikt — Codex, Gemini CLI, Aider, OpenCode of elk script.",
    "faq4.q": "Hoe werken de meldingen?", "faq4.a": 'Shell-integratiescripts detecteren wanneer een commando eindigt of wordt onderbroken. Het paneel krijgt een blauwe ring, de sidebar-badge wordt verhoogd en een Windows toast-melding verschijnt. Ondersteunt OSC 9/99/777, de CLI <code>pandamux notify</code> en inactiviteitsdetectie.',
    "faq5.q": "Hoe verschilt het van Windows Terminal?", "faq5.a": "Windows Terminal heeft geen meldingssysteem, geen agentactiviteitsweergave en geen live-metadata in tabbladen. PandaMUX is specifiek ontworpen voor het toezicht op AI-agenten.",
    "faq6.q": "Is het gratis?", "faq6.a": 'Ja. PandaMUX is open-source onder MIT. Gratis download op <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Downloaden voor Windows", "cta.github": "Bekijk op GitHub",
    "footer.product": "Product", "footer.changelog": "Changelog", "footer.community": "Community", "footer.releases": "Releases",
    "footer.resources": "Bronnen", "footer.docs": "Documentatie", "footer.install": "Installatie", "footer.shortcuts": "Sneltoetsen",
    "footer.legal": "Juridisch", "footer.license": "MIT-licentie", "footer.social": "Social", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windows-fork van", "plugin.badge": "Claude Code Plugin", "plugin.desc": "Start meerdere Claude Code-agenten parallel vanuit één commando. De plugin analyseert je codebase, splitst de taak in onafhankelijke deeltaken en verdeelt ze in golven met afhankelijkheidsbeheer, geïsoleerde bestandszones en automatische review aan het einde van elke cyclus.", "plugin.install_label": "Installatie", "plugin.view_github": "Bekijk op GitHub", "plugin.docs": "Documentatie",
  },

  // ═══════════════════════════ POLISH ═══════════════════════════
  pl: {
    "nav.docs": "Docs", "nav.changelog": "Zmiany", "nav.community": "Społeczność", "nav.github": "GitHub",
    "header.download": "Pobierz dla Windows",
    "hero.tagline": "Terminal stworzony dla", "hero.word1": "Claude Code", "hero.word2": "agentów AI", "hero.word3": "wielozadaniowości", "hero.word4": "Windows",
    "hero.desc": "Natywna aplikacja Windows napisana w Rust. Pionowe karty, powiadomienia gdy agenci potrzebują uwagi, podzielone panele, zdalne sesje SSH i API pipe do automatyzacji.",
    "hero.download": "Pobierz dla Windows", "hero.github": "Zobacz na GitHub",
    "features.title": "Funkcje",
    "f1.name": "Pasywna integracja z Claude Code", "f1.desc": "PandaMUX obserwuje Claude Code bez modyfikowania jego zachowania. Automatycznie konfigurowane hooki raportują aktywność agentów i narzędzi na pasku bocznym. Zero konfiguracji.",
    "f2.name": "Pierścienie powiadomień", "f2.desc": "panele świecą na niebiesko gdy agenci potrzebują uwagi. Powiadomienia toast Windows, flash paska zadań i wbudowane centrum powiadomień.",
    "f3.name": "Pionowe karty", "f3.desc": "pasek boczny wyświetla branch git, katalog roboczy, porty, liczbę agentów, status PR i tekst powiadomienia.",
    "f4.name": "Podzielone panele", "f4.desc": "podziały poziome i pionowe w każdej karcie. Ctrl+D aby podzielić w prawo, Ctrl+Shift+D aby podzielić w dół.",
    "f5.name": "Wskaźniki aktywności", "f5.desc": "pulsujący pomarańczowy = pracuje, zielony = gotowe, czerwony = przerwane.",
    "f6.name": "Zapisane sesje", "f6.desc": "zapisz swoje podziały i katalogi. Automatyczne przywracanie przy uruchomieniu.",
    "f7.name": "Skryptowalny", "f7.desc": 'CLI i API named pipe (<code>\\\\.\\pipe\\pandamux</code>) do automatyzacji i skryptów. Protokół JSON-RPC v2 kompatybilny z cmux.',
    "f8.name": "Natywny Windows", "f8.desc": "ConPTY do emulacji terminala, powiadomienia toast Windows, flash paska zadań.",
    "f9.name": "Akceleracja GPU", "f9.desc": "napędzany przez wgpu z renderowaniem GPU.",
    "f10.name": "Kompatybilne motywy", "f10.desc": "importuj motywy z Windows Terminal lub Ghostty. 450+ motywów Ghostty w zestawie.",
    "f11.name": "Skróty klawiszowe", "f11.desc": "pełne skróty dla przestrzeni roboczych, podziałów, sesji SSH i więcej.",
    "faq.title": "Często zadawane pytania",
    "faq1.q": "Jaka jest relacja między PandaMUX a cmux?", "faq1.a": 'PandaMUX to fork Windows projektu <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux to natywna aplikacja macOS. PandaMUX odtwarza to samo doświadczenie na Windows.',
    "faq2.q": "Jakie platformy są obsługiwane?", "faq2.a": 'Tylko Windows 10 i 11 (x64). Dla macOS użyj <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Jakie agenty są kompatybilne?", "faq3.a": "PandaMUX jest zoptymalizowany dla Claude Code. Każdy agent wiersza poleceń może być używany — Codex, Gemini CLI, Aider, OpenCode lub dowolny skrypt.",
    "faq4.q": "Jak działają powiadomienia?", "faq4.a": 'Skrypty integracji shell wykrywają zakończenie lub przerwanie polecenia. Panel otrzymuje niebieski pierścień, badge paska bocznego się zwiększa i pojawia się powiadomienie toast. Obsługuje OSC 9/99/777 i CLI <code>pandamux notify</code>.',
    "faq5.q": "Czym się różni od Windows Terminal?", "faq5.a": "Windows Terminal nie ma systemu powiadomień, widoczności agentów ani metadanych na żywo. PandaMUX jest zaprojektowany do nadzorowania agentów AI.",
    "faq6.q": "Czy jest darmowy?", "faq6.a": 'Tak. PandaMUX jest open-source na licencji MIT. Darmowe pobieranie na <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Pobierz dla Windows", "cta.github": "Zobacz na GitHub",
    "footer.product": "Produkt", "footer.changelog": "Zmiany", "footer.community": "Społeczność", "footer.releases": "Wydania",
    "footer.resources": "Zasoby", "footer.docs": "Dokumentacja", "footer.install": "Instalacja", "footer.shortcuts": "Skróty",
    "footer.legal": "Prawne", "footer.license": "Licencja MIT", "footer.social": "Social", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Fork Windows projektu", "plugin.badge": "Plugin Claude Code", "plugin.desc": "Uruchom wielu agentów Claude Code równolegle jednym poleceniem. Plugin analizuje bazę kodu, dzieli zadanie na niezależne podzadania i rozdziela je falami z zarządzaniem zależnościami, izolowanymi strefami plików i automatycznym przeglądem na końcu każdego cyklu.", "plugin.install_label": "Instalacja", "plugin.view_github": "Zobacz na GitHub", "plugin.docs": "Dokumentacja",
  },

  // ═══════════════════════════ TURKISH ═══════════════════════════
  tr: {
    "nav.docs": "Docs", "nav.changelog": "Değişiklikler", "nav.community": "Topluluk", "nav.github": "GitHub",
    "header.download": "Windows için indir",
    "hero.tagline": "Şunun için tasarlanan terminal:", "hero.word1": "Claude Code", "hero.word2": "yapay zeka ajanları", "hero.word3": "çoklu görev", "hero.word4": "Windows",
    "hero.desc": "Rust ile yazılmış yerel Windows uygulaması. Dikey sekmeler, ajanlar ilgi istediğinde bildirimler, bölünmüş paneller, uzak SSH oturumları ve otomasyon için pipe API.",
    "hero.download": "Windows için indir", "hero.github": "GitHub'da görüntüle",
    "features.title": "Özellikler",
    "f1.name": "Pasif Claude Code entegrasyonu", "f1.desc": "PandaMUX, Claude Code'u davranışını değiştirmeden gözlemler. Otomatik yapılandırılan hook'lar ajan ve araç etkinliğini kenar çubuğuna bildirir. Sıfır yapılandırma.",
    "f2.name": "Bildirim halkaları", "f2.desc": "ajanlar ilgi istediğinde paneller mavi yanar. Windows toast bildirimi, görev çubuğu flash ve yerleşik bildirim merkezi.",
    "f3.name": "Dikey sekmeler", "f3.desc": "kenar çubuğu git branch, çalışma dizini, portlar, ajan sayısı, PR durumu ve bildirim metnini gösterir.",
    "f4.name": "Bölünmüş paneller", "f4.desc": "her sekmede yatay ve dikey bölmeler. Sağa bölmek için Ctrl+D, aşağı bölmek için Ctrl+Shift+D.",
    "f5.name": "Aktivite göstergeleri", "f5.desc": "titreşen turuncu = çalışıyor, yeşil = tamamlandı, kırmızı = kesildi.",
    "f6.name": "Kayıtlı oturumlar", "f6.desc": "bölmelerinizi ve dizinlerinizi kaydedin. Başlangıçta otomatik geri yükleme.",
    "f7.name": "Scriptlenebilir", "f7.desc": 'CLI ve named pipe API (<code>\\\\.\\pipe\\pandamux</code>) otomasyon ve scripting için. cmux ile uyumlu JSON-RPC v2 protokolü.',
    "f8.name": "Windows doğal", "f8.desc": "terminal emülasyonu için ConPTY, Windows toast bildirimleri, görev çubuğu flash.",
    "f9.name": "GPU hızlandırma", "f9.desc": "akıcı görüntüleme için GPU render ile wgpu tarafından desteklenir.",
    "f10.name": "Tema uyumlu", "f10.desc": "Windows Terminal veya Ghostty'den temalarınızı içe aktarın. 450+ Ghostty teması dahil.",
    "f11.name": "Klavye kısayolları", "f11.desc": "çalışma alanları, bölmeler, SSH oturumları ve daha fazlası için tam kısayollar.",
    "faq.title": "Sık sorulan sorular",
    "faq1.q": "PandaMUX ve cmux arasındaki ilişki nedir?", "faq1.a": 'PandaMUX, <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>\'un Windows fork\'udur. cmux yerel macOS uygulamasıdır. PandaMUX aynı deneyimi Windows\'ta Rust + Iced + alacritty_terminal + ConPTY kullanarak yeniden üretir.',
    "faq2.q": "Hangi platformlar destekleniyor?", "faq2.a": 'Yalnızca Windows 10 ve 11 (x64). macOS için <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a> kullanın.',
    "faq3.q": "Hangi ajanlar uyumlu?", "faq3.a": "PandaMUX, Claude Code için optimize edilmiştir. Herhangi bir komut satırı ajanı kullanılabilir — Codex, Gemini CLI, Aider, OpenCode veya herhangi bir script.",
    "faq4.q": "Bildirimler nasıl çalışır?", "faq4.a": 'Shell entegrasyon scriptleri bir komutun bittiğini veya kesildiğini algılar. Panel mavi halka alır, kenar çubuğu rozeti artar ve Windows toast bildirimi görünür. OSC 9/99/777, <code>pandamux notify</code> CLI ve boşta algılamayı destekler.',
    "faq5.q": "Windows Terminal'den farkı nedir?", "faq5.a": "Windows Terminal'de bildirim sistemi, ajan aktivite görünürlüğü ve canlı metadata yoktur. PandaMUX özellikle AI ajanlarını denetlemek için tasarlanmıştır.",
    "faq6.q": "Ücretsiz mi?", "faq6.a": 'Evet. PandaMUX MIT lisansı altında açık kaynaklıdır. <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>\'tan ücretsiz indirin.',
    "cta.download": "Windows için indir", "cta.github": "GitHub'da görüntüle",
    "footer.product": "Ürün", "footer.changelog": "Değişiklikler", "footer.community": "Topluluk", "footer.releases": "Sürümler",
    "footer.resources": "Kaynaklar", "footer.docs": "Belgeler", "footer.install": "Kurulum", "footer.shortcuts": "Kısayollar",
    "footer.legal": "Hukuki", "footer.license": "MIT Lisansı", "footer.social": "Sosyal", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Şunun Windows fork'u:", "plugin.badge": "Claude Code Eklentisi", "plugin.desc": "Tek bir komutla birden fazla Claude Code ajanını paralel olarak başlatın. Eklenti kod tabanınızı analiz eder, görevi bağımsız alt görevlere böler ve bunları bağımlılık yönetimi, izole dosya bölgeleri ve her döngü sonunda otomatik inceleme ile dalgalar halinde dağıtır.", "plugin.install_label": "Kurulum", "plugin.view_github": "GitHub'da Görüntüle", "plugin.docs": "Dokümantasyon",
  },

  // ═══════════════════════════ RUSSIAN ═══════════════════════════
  ru: {
    "nav.docs": "Документация", "nav.changelog": "Изменения", "nav.community": "Сообщество", "nav.github": "GitHub",
    "header.download": "Скачать для Windows",
    "hero.tagline": "Терминал, созданный для", "hero.word1": "Claude Code", "hero.word2": "ИИ-агентов", "hero.word3": "многозадачности", "hero.word4": "Windows",
    "hero.desc": "Нативное приложение для Windows, написанное на Rust. Вертикальные вкладки, уведомления когда агентам нужно внимание, разделённые панели, удалённые сессии SSH и API через пайп для автоматизации.",
    "hero.download": "Скачать для Windows", "hero.github": "Смотреть на GitHub",
    "features.title": "Возможности",
    "f1.name": "Пассивная интеграция с Claude Code", "f1.desc": "PandaMUX наблюдает за Claude Code не изменяя его поведение. Автоматически настроенные хуки сообщают об активности агентов и инструментов на боковой панели. Нулевая настройка.",
    "f2.name": "Кольца уведомлений", "f2.desc": "панели подсвечиваются синим когда агентам нужно внимание. Toast-уведомления Windows, мигание панели задач и встроенный центр уведомлений.",
    "f3.name": "Вертикальные вкладки", "f3.desc": "боковая панель показывает ветку git, рабочий каталог, порты, количество агентов, статус PR и текст уведомления.",
    "f4.name": "Разделённые панели", "f4.desc": "горизонтальные и вертикальные разделения в каждой вкладке. Ctrl+D для разделения вправо, Ctrl+Shift+D вниз.",
    "f5.name": "Индикаторы активности", "f5.desc": "пульсирующий оранжевый = работает, зелёный = готово, красный = прервано.",
    "f6.name": "Сохранённые сессии", "f6.desc": "сохраняйте разделения и каталоги. Автоматическое восстановление при запуске.",
    "f7.name": "Скриптуемый", "f7.desc": 'CLI и API именованного канала (<code>\\\\.\\pipe\\pandamux</code>) для автоматизации и скриптов. Протокол JSON-RPC v2 совместимый с cmux.',
    "f8.name": "Нативный для Windows", "f8.desc": "ConPTY для эмуляции терминала, toast-уведомления Windows, мигание панели задач.",
    "f9.name": "Ускорение GPU", "f9.desc": "работает на wgpu с рендерингом GPU для плавного отображения.",
    "f10.name": "Совместимость тем", "f10.desc": "импортируйте темы из Windows Terminal или Ghostty. 450+ тем Ghostty в комплекте.",
    "f11.name": "Горячие клавиши", "f11.desc": "полный набор горячих клавиш для рабочих пространств, разделений, сессий SSH и прочего.",
    "faq.title": "Часто задаваемые вопросы",
    "faq1.q": "Какая связь между PandaMUX и cmux?", "faq1.a": 'PandaMUX — это форк <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a> для Windows. cmux — нативное приложение macOS. PandaMUX воспроизводит тот же опыт на Windows используя Rust + Iced + alacritty_terminal + ConPTY.',
    "faq2.q": "Какие платформы поддерживаются?", "faq2.a": 'Только Windows 10 и 11 (x64). Для macOS используйте <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Какие агенты совместимы?", "faq3.a": "PandaMUX оптимизирован для Claude Code. Любой агент командной строки может использоваться — Codex, Gemini CLI, Aider, OpenCode или любой скрипт.",
    "faq4.q": "Как работают уведомления?", "faq4.a": 'Скрипты интеграции оболочки определяют завершение или прерывание команды. Панель получает синее кольцо, бейдж боковой панели увеличивается и появляется toast-уведомление. Поддерживает OSC 9/99/777, CLI <code>pandamux notify</code> и обнаружение простоя.',
    "faq5.q": "Чем отличается от Windows Terminal?", "faq5.a": "Windows Terminal не имеет системы уведомлений, видимости активности агентов и метаданных в реальном времени. PandaMUX разработан специально для наблюдения за ИИ-агентами.",
    "faq6.q": "Это бесплатно?", "faq6.a": 'Да. PandaMUX — open-source под лицензией MIT. Бесплатная загрузка на <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Скачать для Windows", "cta.github": "Смотреть на GitHub",
    "footer.product": "Продукт", "footer.changelog": "Изменения", "footer.community": "Сообщество", "footer.releases": "Релизы",
    "footer.resources": "Ресурсы", "footer.docs": "Документация", "footer.install": "Установка", "footer.shortcuts": "Горячие клавиши",
    "footer.legal": "Правовая информация", "footer.license": "Лицензия MIT", "footer.social": "Социальные сети", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Форк Windows проекта", "plugin.badge": "Плагин Claude Code", "plugin.desc": "Запускайте несколько агентов Claude Code параллельно одной командой. Плагин анализирует кодовую базу, разбивает задачу на независимые подзадачи и распределяет их волнами с управлением зависимостями, изолированными файловыми зонами и автоматической проверкой в конце каждого цикла.", "plugin.install_label": "Установка", "plugin.view_github": "Смотреть на GitHub", "plugin.docs": "Документация",
  },

  // ═══════════════════════════ UKRAINIAN ═══════════════════════════
  uk: {
    "nav.docs": "Документація", "nav.changelog": "Зміни", "nav.community": "Спільнота", "nav.github": "GitHub",
    "header.download": "Завантажити для Windows",
    "hero.tagline": "Термінал, створений для", "hero.word1": "Claude Code", "hero.word2": "ШІ-агентів", "hero.word3": "багатозадачності", "hero.word4": "Windows",
    "hero.desc": "Нативний застосунок для Windows, написаний на Rust. Вертикальні вкладки, сповіщення коли агентам потрібна увага, розділені панелі, віддалені сесії SSH та API через пайп для автоматизації.",
    "hero.download": "Завантажити для Windows", "hero.github": "Дивитися на GitHub",
    "features.title": "Можливості",
    "f1.name": "Пасивна інтеграція з Claude Code", "f1.desc": "PandaMUX спостерігає за Claude Code не змінюючи його поведінку. Автоматично налаштовані хуки повідомляють про активність агентів та інструментів на бічній панелі.",
    "f2.name": "Кільця сповіщень", "f2.desc": "панелі підсвічуються синім коли агентам потрібна увага. Toast-сповіщення Windows та центр сповіщень.",
    "f3.name": "Вертикальні вкладки", "f3.desc": "бічна панель показує гілку git, робочий каталог, порти, кількість агентів, статус PR.",
    "f4.name": "Розділені панелі", "f4.desc": "горизонтальні та вертикальні розділення в кожній вкладці.",
    "f5.name": "Індикатори активності", "f5.desc": "пульсуючий помаранчевий = працює, зелений = готово, червоний = перервано.",
    "f6.name": "Збережені сесії", "f6.desc": "зберігайте розділення та каталоги. Автоматичне відновлення при запуску.",
    "f7.name": "Скриптовий", "f7.desc": 'CLI та API іменованого каналу (<code>\\\\.\\pipe\\pandamux</code>) для автоматизації.',
    "f8.name": "Нативний Windows", "f8.desc": "ConPTY для емуляції терміналу, toast-сповіщення Windows.",
    "f9.name": "Прискорення GPU", "f9.desc": "працює на wgpu з рендерингом GPU.",
    "f10.name": "Сумісність тем", "f10.desc": "імпортуйте теми з Windows Terminal або Ghostty. 450+ тем в комплекті.",
    "f11.name": "Гарячі клавіші", "f11.desc": "повний набір гарячих клавіш для робочих просторів, розділень, сесій SSH.",
    "faq.title": "Поширені запитання",
    "faq1.q": "Який зв'язок між PandaMUX і cmux?", "faq1.a": 'PandaMUX — це форк <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a> для Windows.',
    "faq2.q": "Які платформи підтримуються?", "faq2.a": 'Тільки Windows 10 і 11 (x64). Для macOS використовуйте <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Які агенти сумісні?", "faq3.a": "PandaMUX оптимізований для Claude Code. Будь-який агент командного рядка може використовуватися.",
    "faq4.q": "Як працюють сповіщення?", "faq4.a": 'Скрипти інтеграції оболонки виявляють завершення або переривання команди. Підтримує OSC 9/99/777, CLI <code>pandamux notify</code>.',
    "faq5.q": "Чим відрізняється від Windows Terminal?", "faq5.a": "PandaMUX розроблений спеціально для спостереження за ШІ-агентами зі сповіщеннями та живими метаданими.",
    "faq6.q": "Це безкоштовно?", "faq6.a": 'Так. PandaMUX — open-source під ліцензією MIT. <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Завантажити для Windows", "cta.github": "Дивитися на GitHub",
    "footer.product": "Продукт", "footer.changelog": "Зміни", "footer.community": "Спільнота", "footer.releases": "Релізи",
    "footer.resources": "Ресурси", "footer.docs": "Документація", "footer.install": "Встановлення", "footer.shortcuts": "Гарячі клавіші",
    "footer.legal": "Правова інформація", "footer.license": "Ліцензія MIT", "footer.social": "Соціальні мережі", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Форк Windows проєкту", "plugin.badge": "Плагін Claude Code", "plugin.desc": "Запускайте кілька агентів Claude Code паралельно однією командою. Плагін аналізує кодову базу, розбиває завдання на незалежні підзавдання та розподіляє їх хвилями з управлінням залежностями, ізольованими файловими зонами та автоматичною перевіркою наприкінці кожного циклу.", "plugin.install_label": "Встановлення", "plugin.view_github": "Переглянути на GitHub", "plugin.docs": "Документація",
  },

  // ═══════════════════════════ ARABIC ═══════════════════════════
  ar: {
    "nav.docs": "المستندات", "nav.changelog": "سجل التغييرات", "nav.community": "المجتمع", "nav.github": "GitHub",
    "header.download": "تحميل لـ Windows",
    "hero.tagline": "الطرفية المصممة لـ", "hero.word1": "Claude Code", "hero.word2": "وكلاء الذكاء الاصطناعي", "hero.word3": "تعدد المهام", "hero.word4": "Windows",
    "hero.desc": "تطبيق Windows أصلي مكتوب بلغة Rust. علامات تبويب عمودية، إشعارات عندما يحتاج الوكلاء للانتباه، ألواح مقسمة، جلسات SSH عن بعد وواجهة برمجة تطبيقات أنابيب للأتمتة.",
    "hero.download": "تحميل لـ Windows", "hero.github": "عرض على GitHub",
    "features.title": "الميزات",
    "f1.name": "تكامل سلبي مع Claude Code", "f1.desc": "يراقب PandaMUX Claude Code دون تغيير سلوكه. الخطافات المهيأة تلقائيًا تُبلغ عن نشاط الوكلاء والأدوات في الشريط الجانبي. بدون أي إعداد.",
    "f2.name": "حلقات الإشعارات", "f2.desc": "تتوهج الألواح باللون الأزرق عندما يحتاج الوكلاء للانتباه. إشعارات Windows ومركز إشعارات مدمج.",
    "f3.name": "علامات تبويب عمودية", "f3.desc": "الشريط الجانبي يعرض فرع git، دليل العمل، المنافذ، عدد الوكلاء وحالة PR.",
    "f4.name": "ألواح مقسمة", "f4.desc": "تقسيمات أفقية وعمودية في كل علامة تبويب. Ctrl+D للتقسيم يمينًا، Ctrl+Shift+D للتقسيم لأسفل.",
    "f5.name": "مؤشرات النشاط", "f5.desc": "برتقالي نابض = يعمل، أخضر = انتهى، أحمر = توقف.",
    "f6.name": "جلسات محفوظة", "f6.desc": "احفظ التقسيمات والأدلة. استعادة تلقائية عند بدء التشغيل.",
    "f7.name": "قابل للبرمجة", "f7.desc": 'CLI وواجهة أنابيب مسماة (<code>\\\\.\\pipe\\pandamux</code>) للأتمتة. بروتوكول JSON-RPC v2 متوافق مع cmux.',
    "f8.name": "أصلي لـ Windows", "f8.desc": "ConPTY لمحاكاة الطرفية، إشعارات Windows، وميض شريط المهام.",
    "f9.name": "تسريع GPU", "f9.desc": "مدعوم بـ wgpu مع عرض GPU لعرض سلس.",
    "f10.name": "متوافق مع السمات", "f10.desc": "استورد سماتك من Windows Terminal أو Ghostty. أكثر من 450 سمة Ghostty مضمنة.",
    "f11.name": "اختصارات لوحة المفاتيح", "f11.desc": "اختصارات كاملة لمساحات العمل والتقسيمات وجلسات SSH والمزيد.",
    "faq.title": "الأسئلة الشائعة",
    "faq1.q": "ما العلاقة بين PandaMUX و cmux؟", "faq1.a": 'PandaMUX هو فرع Windows من <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>. cmux تطبيق macOS أصلي. PandaMUX يعيد إنتاج نفس التجربة على Windows.',
    "faq2.q": "ما المنصات المدعومة؟", "faq2.a": 'Windows 10 و 11 فقط (x64). لـ macOS استخدم <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "ما الوكلاء المتوافقون؟", "faq3.a": "PandaMUX مُحسَّن لـ Claude Code. يمكن استخدام أي وكيل سطر أوامر — Codex، Gemini CLI، Aider، OpenCode أو أي برنامج نصي.",
    "faq4.q": "كيف تعمل الإشعارات؟", "faq4.a": 'تكتشف نصوص تكامل الصدفة انتهاء أو مقاطعة الأمر. يدعم OSC 9/99/777 و <code>pandamux notify</code>.',
    "faq5.q": "كيف يختلف عن Windows Terminal؟", "faq5.a": "PandaMUX مصمم خصيصًا لمراقبة وكلاء الذكاء الاصطناعي مع إشعارات وبيانات وصفية حية.",
    "faq6.q": "هل هو مجاني؟", "faq6.a": 'نعم. PandaMUX مفتوح المصدر تحت رخصة MIT. تحميل مجاني من <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "تحميل لـ Windows", "cta.github": "عرض على GitHub",
    "footer.product": "المنتج", "footer.changelog": "سجل التغييرات", "footer.community": "المجتمع", "footer.releases": "الإصدارات",
    "footer.resources": "الموارد", "footer.docs": "التوثيق", "footer.install": "التثبيت", "footer.shortcuts": "الاختصارات",
    "footer.legal": "قانوني", "footer.license": "رخصة MIT", "footer.social": "التواصل", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "فرع Windows من", "plugin.badge": "إضافة Claude Code", "plugin.desc": "شغّل عدة وكلاء Claude Code بالتوازي بأمر واحد. تحلل الإضافة قاعدة الشيفرة وتقسّم المهمة إلى مهام فرعية مستقلة وتوزعها على موجات مع إدارة التبعيات ومناطق ملفات معزولة ومراجعة تلقائية في نهاية كل دورة.", "plugin.install_label": "التثبيت", "plugin.view_github": "عرض على GitHub", "plugin.docs": "التوثيق",
  },

  // ═══════════════════════════ CHINESE SIMPLIFIED ═══════════════════════════
  zh: {
    "nav.docs": "文档", "nav.changelog": "更新日志", "nav.community": "社区", "nav.github": "GitHub",
    "header.download": "下载 Windows 版",
    "hero.tagline": "专为以下场景打造的终端：", "hero.word1": "Claude Code", "hero.word2": "AI 代理", "hero.word3": "多任务处理", "hero.word4": "Windows",
    "hero.desc": "使用 Rust 编写的原生 Windows 应用。垂直标签页、代理需要关注时发送通知、分屏面板、SSH 远程会话和用于自动化的管道 API。",
    "hero.download": "下载 Windows 版", "hero.github": "在 GitHub 上查看",
    "features.title": "功能特性",
    "f1.name": "Claude Code 被动集成", "f1.desc": "PandaMUX 在不改变 Claude Code 行为的情况下进行观察。自动配置的钩子将代理和工具活动报告到侧边栏。零配置。",
    "f2.name": "通知环", "f2.desc": "当代理需要关注时面板会发蓝光。Windows toast 通知、任务栏闪烁和内置通知中心。",
    "f3.name": "垂直标签页", "f3.desc": "侧边栏显示 git 分支、工作目录、端口、代理数量、PR 状态和通知文本。",
    "f4.name": "分屏面板", "f4.desc": "每个标签页中可水平和垂直分屏。Ctrl+D 向右分屏，Ctrl+Shift+D 向下分屏。",
    "f5.name": "活动指示器", "f5.desc": "脉冲橙色 = 工作中，绿色 = 完成，红色 = 中断。",
    "f6.name": "保存会话", "f6.desc": "保存分屏布局和目录。启动时自动恢复。",
    "f7.name": "可脚本化", "f7.desc": 'CLI 和命名管道 API（<code>\\\\.\\pipe\\pandamux</code>）用于自动化和脚本。兼容 cmux 的 JSON-RPC v2 协议。',
    "f8.name": "Windows 原生", "f8.desc": "ConPTY 终端模拟、Windows toast 通知、任务栏闪烁。",
    "f9.name": "GPU 加速", "f9.desc": "由 wgpu 的 GPU 渲染驱动，显示流畅。",
    "f10.name": "主题兼容", "f10.desc": "从 Windows Terminal 或 Ghostty 导入主题。内置 450+ Ghostty 主题。",
    "f11.name": "键盘快捷键", "f11.desc": "工作区、分屏、SSH 会话等完整快捷键。",
    "faq.title": "常见问题",
    "faq1.q": "PandaMUX 和 cmux 是什么关系？", "faq1.a": 'PandaMUX 是 <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a> 的 Windows 分支。cmux 是原生 macOS 应用。PandaMUX 使用 Rust + Iced + alacritty_terminal + ConPTY 在 Windows 上重现相同体验。',
    "faq2.q": "支持哪些平台？", "faq2.a": '仅 Windows 10 和 11（x64）。macOS 请使用 <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>。',
    "faq3.q": "哪些代理兼容？", "faq3.a": "PandaMUX 针对 Claude Code 进行了优化。任何命令行代理都可使用 — Codex、Gemini CLI、Aider、OpenCode 或任何脚本。",
    "faq4.q": "通知如何工作？", "faq4.a": 'Shell 集成脚本检测命令完成或中断。支持 OSC 9/99/777、CLI <code>pandamux notify</code> 和空闲检测。',
    "faq5.q": "与 Windows Terminal 有何不同？", "faq5.a": "PandaMUX 专为监督 AI 代理而设计，具有通知系统和实时元数据。",
    "faq6.q": "免费吗？", "faq6.a": '是的。PandaMUX 是 MIT 许可的开源项目。在 <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a> 免费下载。',
    "cta.download": "下载 Windows 版", "cta.github": "在 GitHub 上查看",
    "footer.product": "产品", "footer.changelog": "更新日志", "footer.community": "社区", "footer.releases": "发布",
    "footer.resources": "资源", "footer.docs": "文档", "footer.install": "安装", "footer.shortcuts": "快捷键",
    "footer.legal": "法律", "footer.license": "MIT 许可证", "footer.social": "社交", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windows 分支自", "plugin.badge": "Claude Code 插件", "plugin.desc": "通过一条命令并行启动多个 Claude Code 代理。插件会分析代码库，将任务拆分为独立的子任务，并以波次方式分配，支持依赖管理、隔离文件区域和每轮结束时的自动审查。", "plugin.install_label": "安装", "plugin.view_github": "在 GitHub 上查看", "plugin.docs": "文档",
  },

  // ═══════════════════════════ CHINESE TRADITIONAL ═══════════════════════════
  "zh-TW": {
    "nav.docs": "文件", "nav.changelog": "更新日誌", "nav.community": "社群", "nav.github": "GitHub",
    "header.download": "下載 Windows 版",
    "hero.tagline": "專為以下場景打造的終端：", "hero.word1": "Claude Code", "hero.word2": "AI 代理", "hero.word3": "多工處理", "hero.word4": "Windows",
    "hero.desc": "使用 Rust 編寫的原生 Windows 應用。垂直分頁、代理需要關注時發送通知、分割面板、SSH 遠端工作階段和用於自動化的管道 API。",
    "hero.download": "下載 Windows 版", "hero.github": "在 GitHub 上查看",
    "features.title": "功能特性",
    "f1.name": "Claude Code 被動整合", "f1.desc": "PandaMUX 在不改變 Claude Code 行為的情況下進行觀察。自動配置的掛鉤將代理和工具活動回報至側邊欄。",
    "f2.name": "通知環", "f2.desc": "當代理需要關注時面板會發藍光。Windows toast 通知和內建通知中心。",
    "f3.name": "垂直分頁", "f3.desc": "側邊欄顯示 git 分支、工作目錄、連接埠、代理數量、PR 狀態。",
    "f4.name": "分割面板", "f4.desc": "每個分頁中可水平和垂直分割。",
    "f5.name": "活動指示器", "f5.desc": "脈衝橘色 = 工作中，綠色 = 完成，紅色 = 中斷。",
    "f6.name": "儲存工作階段", "f6.desc": "儲存分割佈局和目錄。啟動時自動恢復。",
    "f7.name": "可腳本化", "f7.desc": 'CLI 和命名管道 API（<code>\\\\.\\pipe\\pandamux</code>）用於自動化。',
    "f8.name": "Windows 原生", "f8.desc": "ConPTY 終端模擬、Windows toast 通知。",
    "f9.name": "GPU 加速", "f9.desc": "由 wgpu 的 GPU 渲染驅動。",
    "f10.name": "佈景主題相容", "f10.desc": "從 Windows Terminal 或 Ghostty 匯入佈景主題。內含 450+ Ghostty 佈景主題。",
    "f11.name": "鍵盤快速鍵", "f11.desc": "工作區、分割、SSH 工作階段等完整快速鍵。",
    "faq.title": "常見問題",
    "faq1.q": "PandaMUX 和 cmux 是什麼關係？", "faq1.a": 'PandaMUX 是 <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a> 的 Windows 分支。',
    "faq2.q": "支援哪些平台？", "faq2.a": '僅 Windows 10 和 11（x64）。macOS 請使用 <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>。',
    "faq3.q": "哪些代理相容？", "faq3.a": "PandaMUX 針對 Claude Code 進行了最佳化。任何命令列代理皆可使用。",
    "faq4.q": "通知如何運作？", "faq4.a": 'Shell 整合腳本偵測命令完成或中斷。支援 OSC 9/99/777 和 <code>pandamux notify</code>。',
    "faq5.q": "與 Windows Terminal 有何不同？", "faq5.a": "PandaMUX 專為監督 AI 代理而設計。",
    "faq6.q": "免費嗎？", "faq6.a": '是的。PandaMUX 是 MIT 授權的開源專案。<a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>。',
    "cta.download": "下載 Windows 版", "cta.github": "在 GitHub 上查看",
    "footer.product": "產品", "footer.changelog": "更新日誌", "footer.community": "社群", "footer.releases": "發布",
    "footer.resources": "資源", "footer.docs": "文件", "footer.install": "安裝", "footer.shortcuts": "快速鍵",
    "footer.legal": "法律", "footer.license": "MIT 授權", "footer.social": "社群", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windows 分支自", "plugin.badge": "Claude Code 外掛", "plugin.desc": "透過一條指令並行啟動多個 Claude Code 代理。外掛會分析程式碼庫，將任務拆分為獨立的子任務，並以波次方式分配，支援依賴管理、隔離檔案區域和每輪結束時的自動審查。", "plugin.install_label": "安裝", "plugin.view_github": "在 GitHub 上查看", "plugin.docs": "文件",
  },

  // ═══════════════════════════ JAPANESE ═══════════════════════════
  ja: {
    "nav.docs": "ドキュメント", "nav.changelog": "変更履歴", "nav.community": "コミュニティ", "nav.github": "GitHub",
    "header.download": "Windows版をダウンロード",
    "hero.tagline": "次のために設計されたターミナル：", "hero.word1": "Claude Code", "hero.word2": "AIエージェント", "hero.word3": "マルチタスク", "hero.word4": "Windows",
    "hero.desc": "Rustで書かれたネイティブWindowsアプリケーション。垂直タブ、エージェントが注意を必要とする時の通知、分割ペイン、SSHリモートセッション、自動化のためのパイプAPI。",
    "hero.download": "Windows版をダウンロード", "hero.github": "GitHubで見る",
    "features.title": "機能",
    "f1.name": "Claude Codeパッシブ統合", "f1.desc": "PandaMUXはClaude Codeの動作を変更せずに監視します。自動設定されたフックがエージェントとツールの活動をサイドバーに報告。設定不要。",
    "f2.name": "通知リング", "f2.desc": "エージェントが注意を必要とするとパネルが青く光ります。Windowsトースト通知とタスクバーフラッシュ。",
    "f3.name": "垂直タブ", "f3.desc": "サイドバーにgitブランチ、作業ディレクトリ、ポート、エージェント数、PRステータスを表示。",
    "f4.name": "分割ペイン", "f4.desc": "各タブで水平・垂直分割。Ctrl+Dで右に分割、Ctrl+Shift+Dで下に分割。",
    "f5.name": "アクティビティインジケーター", "f5.desc": "パルスオレンジ = 作業中、緑 = 完了、赤 = 中断。",
    "f6.name": "セッション保存", "f6.desc": "分割とディレクトリを保存。起動時に自動復元。",
    "f7.name": "スクリプト可能", "f7.desc": 'CLIと名前付きパイプAPI（<code>\\\\.\\pipe\\pandamux</code>）で自動化。cmux互換のJSON-RPC v2プロトコル。',
    "f8.name": "Windowsネイティブ", "f8.desc": "ConPTYターミナルエミュレーション、Windowsトースト通知。",
    "f9.name": "GPUアクセラレーション", "f9.desc": "wgpuのGPUレンダリングによるスムーズな表示。",
    "f10.name": "テーマ互換", "f10.desc": "Windows TerminalまたはGhosttyからテーマをインポート。450以上のGhosttyテーマ同梱。",
    "f11.name": "キーボードショートカット", "f11.desc": "ワークスペース、分割、SSHセッションなどの完全なショートカット。",
    "faq.title": "よくある質問",
    "faq1.q": "PandaMUXとcmuxの関係は？", "faq1.a": 'PandaMUXは<a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>のWindowsフォークです。cmuxはネイティブmacOSアプリです。',
    "faq2.q": "対応プラットフォームは？", "faq2.a": 'Windows 10と11のみ（x64）。macOSには<a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>をお使いください。',
    "faq3.q": "互換性のあるエージェントは？", "faq3.a": "PandaMUXはClaude Code向けに最適化。任意のコマンドラインエージェントが使用可能です。",
    "faq4.q": "通知はどう機能しますか？", "faq4.a": 'シェル統合スクリプトがコマンドの完了や中断を検出。OSC 9/99/777、<code>pandamux notify</code> CLIに対応。',
    "faq5.q": "Windows Terminalとの違いは？", "faq5.a": "PandaMUXはAIエージェントの監視に特化しており、通知システムとリアルタイムメタデータを備えています。",
    "faq6.q": "無料ですか？", "faq6.a": 'はい。PandaMUXはMITライセンスのオープンソースです。<a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>から無料ダウンロード。',
    "cta.download": "Windows版をダウンロード", "cta.github": "GitHubで見る",
    "footer.product": "製品", "footer.changelog": "変更履歴", "footer.community": "コミュニティ", "footer.releases": "リリース",
    "footer.resources": "リソース", "footer.docs": "ドキュメント", "footer.install": "インストール", "footer.shortcuts": "ショートカット",
    "footer.legal": "法的情報", "footer.license": "MITライセンス", "footer.social": "ソーシャル", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windowsフォーク元：", "plugin.badge": "Claude Code プラグイン", "plugin.desc": "1つのコマンドで複数のClaude Codeエージェントを並列起動。プラグインがコードベースを分析し、タスクを独立したサブタスクに分割、依存関係管理・ファイルゾーン分離・サイクル終了時の自動レビューを備えたウェーブで配分します。", "plugin.install_label": "インストール", "plugin.view_github": "GitHubで見る", "plugin.docs": "ドキュメント",
  },

  // ═══════════════════════════ KOREAN ═══════════════════════════
  ko: {
    "nav.docs": "문서", "nav.changelog": "변경 이력", "nav.community": "커뮤니티", "nav.github": "GitHub",
    "header.download": "Windows용 다운로드",
    "hero.tagline": "다음을 위해 만들어진 터미널:", "hero.word1": "Claude Code", "hero.word2": "AI 에이전트", "hero.word3": "멀티태스킹", "hero.word4": "Windows",
    "hero.desc": "Rust로 작성된 네이티브 Windows 애플리케이션. 세로 탭, 에이전트가 주의를 필요로 할 때 알림, 분할 패널, SSH 원격 세션 및 자동화를 위한 파이프 API.",
    "hero.download": "Windows용 다운로드", "hero.github": "GitHub에서 보기",
    "features.title": "기능",
    "f1.name": "Claude Code 패시브 통합", "f1.desc": "PandaMUX는 Claude Code의 동작을 변경하지 않고 관찰합니다. 자동 구성된 훅이 에이전트 및 도구 활동을 사이드바에 보고합니다.",
    "f2.name": "알림 링", "f2.desc": "에이전트가 주의를 필요로 하면 패널이 파란색으로 빛납니다. Windows 토스트 알림 및 작업 표시줄 깜빡임.",
    "f3.name": "세로 탭", "f3.desc": "사이드바에 git 브랜치, 작업 디렉토리, 포트, 에이전트 수, PR 상태를 표시합니다.",
    "f4.name": "분할 패널", "f4.desc": "각 탭에서 가로 및 세로 분할. Ctrl+D로 오른쪽 분할, Ctrl+Shift+D로 아래 분할.",
    "f5.name": "활동 표시기", "f5.desc": "맥동 주황색 = 작업 중, 녹색 = 완료, 빨간색 = 중단.",
    "f6.name": "저장된 세션", "f6.desc": "분할 및 디렉토리를 저장합니다. 시작 시 자동 복원.",
    "f7.name": "스크립팅 가능", "f7.desc": 'CLI 및 명명된 파이프 API (<code>\\\\.\\pipe\\pandamux</code>)로 자동화. cmux 호환 JSON-RPC v2 프로토콜.',
    "f8.name": "Windows 네이티브", "f8.desc": "ConPTY 터미널 에뮬레이션, Windows 토스트 알림, 작업 표시줄 깜빡임.",
    "f9.name": "GPU 가속", "f9.desc": "wgpu의 GPU 렌더링으로 부드러운 표시.",
    "f10.name": "테마 호환", "f10.desc": "Windows Terminal 또는 Ghostty에서 테마를 가져옵니다. 450개 이상의 Ghostty 테마 포함.",
    "f11.name": "키보드 단축키", "f11.desc": "작업 공간, 분할, SSH 세션 등의 전체 단축키.",
    "faq.title": "자주 묻는 질문",
    "faq1.q": "PandaMUX와 cmux의 관계는?", "faq1.a": 'PandaMUX는 <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>의 Windows 포크입니다.',
    "faq2.q": "지원되는 플랫폼은?", "faq2.a": 'Windows 10 및 11만 (x64). macOS는 <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>를 사용하세요.',
    "faq3.q": "호환되는 에이전트는?", "faq3.a": "PandaMUX는 Claude Code에 최적화되어 있습니다. 모든 명령줄 에이전트를 사용할 수 있습니다.",
    "faq4.q": "알림은 어떻게 작동하나요?", "faq4.a": '쉘 통합 스크립트가 명령 완료 또는 중단을 감지합니다. OSC 9/99/777 및 <code>pandamux notify</code> 지원.',
    "faq5.q": "Windows Terminal과의 차이점은?", "faq5.a": "PandaMUX는 알림 시스템 및 실시간 메타데이터를 갖춘 AI 에이전트 감독용으로 설계되었습니다.",
    "faq6.q": "무료인가요?", "faq6.a": '네. PandaMUX는 MIT 라이선스의 오픈소스입니다. <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>에서 무료 다운로드.',
    "cta.download": "Windows용 다운로드", "cta.github": "GitHub에서 보기",
    "footer.product": "제품", "footer.changelog": "변경 이력", "footer.community": "커뮤니티", "footer.releases": "릴리스",
    "footer.resources": "리소스", "footer.docs": "문서", "footer.install": "설치", "footer.shortcuts": "단축키",
    "footer.legal": "법적 정보", "footer.license": "MIT 라이선스", "footer.social": "소셜", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windows 포크 원본:", "plugin.badge": "Claude Code 플러그인", "plugin.desc": "하나의 명령으로 여러 Claude Code 에이전트를 병렬 실행합니다. 플러그인이 코드베이스를 분석하고 작업을 독립적인 하위 작업으로 분할한 뒤 의존성 관리, 격리된 파일 영역, 매 사이클 종료 시 자동 리뷰를 갖춘 웨이브로 배분합니다.", "plugin.install_label": "설치", "plugin.view_github": "GitHub에서 보기", "plugin.docs": "문서",
  },

  // ═══════════════════════════ HINDI ═══════════════════════════
  hi: {
    "nav.docs": "दस्तावेज़", "nav.changelog": "बदलाव", "nav.community": "समुदाय", "nav.github": "GitHub",
    "header.download": "Windows के लिए डाउनलोड करें",
    "hero.tagline": "इसके लिए बनाया गया टर्मिनल:", "hero.word1": "Claude Code", "hero.word2": "AI एजेंट", "hero.word3": "मल्टीटास्किंग", "hero.word4": "Windows",
    "hero.desc": "Rust में लिखा गया नेटिव Windows एप्लिकेशन। वर्टिकल टैब, एजेंट को ध्यान देने की आवश्यकता होने पर सूचनाएं, विभाजित पैनल, SSH रिमोट सत्र और ऑटोमेशन के लिए पाइप API।",
    "hero.download": "Windows के लिए डाउनलोड करें", "hero.github": "GitHub पर देखें",
    "features.title": "विशेषताएं",
    "f1.name": "निष्क्रिय Claude Code एकीकरण", "f1.desc": "PandaMUX बिना व्यवहार बदले Claude Code को देखता है। स्वतः कॉन्फ़िगर किए गए हुक एजेंट और टूल गतिविधि को साइडबार में रिपोर्ट करते हैं।",
    "f2.name": "सूचना रिंग", "f2.desc": "एजेंट को ध्यान देने की जरूरत होने पर पैनल नीले रंग में चमकते हैं।",
    "f3.name": "वर्टिकल टैब", "f3.desc": "साइडबार git ब्रांच, कार्य निर्देशिका, पोर्ट, एजेंट संख्या दिखाता है।",
    "f4.name": "विभाजित पैनल", "f4.desc": "प्रत्येक टैब में क्षैतिज और ऊर्ध्वाधर विभाजन।",
    "f5.name": "गतिविधि संकेतक", "f5.desc": "स्पंदित नारंगी = काम कर रहा है, हरा = पूर्ण, लाल = बाधित।",
    "f6.name": "सहेजे गए सत्र", "f6.desc": "अपने विभाजन और निर्देशिकाएं सहेजें। स्टार्टअप पर स्वचालित पुनर्स्थापना।",
    "f7.name": "स्क्रिप्टेबल", "f7.desc": 'CLI और नामित पाइप API (<code>\\\\.\\pipe\\pandamux</code>) ऑटोमेशन के लिए।',
    "f8.name": "Windows नेटिव", "f8.desc": "ConPTY टर्मिनल एमुलेशन, Windows टोस्ट सूचनाएं।",
    "f9.name": "GPU त्वरण", "f9.desc": "GPU रेंडरिंग के साथ wgpu द्वारा संचालित।",
    "f10.name": "थीम संगत", "f10.desc": "Windows Terminal या Ghostty से थीम आयात करें। 450+ Ghostty थीम शामिल।",
    "f11.name": "कीबोर्ड शॉर्टकट", "f11.desc": "वर्कस्पेस, विभाजन, SSH सत्र और अधिक के लिए पूर्ण शॉर्टकट।",
    "faq.title": "अक्सर पूछे जाने वाले प्रश्न",
    "faq1.q": "PandaMUX और cmux के बीच क्या संबंध है?", "faq1.a": 'PandaMUX, <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a> का Windows फ़ॉर्क है।',
    "faq2.q": "कौन से प्लेटफ़ॉर्म समर्थित हैं?", "faq2.a": 'केवल Windows 10 और 11 (x64)। macOS के लिए <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a> उपयोग करें।',
    "faq3.q": "कौन से एजेंट संगत हैं?", "faq3.a": "PandaMUX Claude Code के लिए अनुकूलित है। कोई भी कमांड-लाइन एजेंट उपयोग किया जा सकता है।",
    "faq4.q": "सूचनाएं कैसे काम करती हैं?", "faq4.a": 'शेल एकीकरण स्क्रिप्ट कमांड पूर्ण होने या बाधित होने का पता लगाती हैं। OSC 9/99/777 और <code>pandamux notify</code> समर्थित।',
    "faq5.q": "Windows Terminal से कैसे अलग है?", "faq5.a": "PandaMUX विशेष रूप से AI एजेंटों की निगरानी के लिए डिज़ाइन किया गया है।",
    "faq6.q": "क्या यह मुफ्त है?", "faq6.a": 'हाँ। PandaMUX MIT लाइसेंस के तहत ओपन-सोर्स है। <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a> से मुफ्त डाउनलोड करें।',
    "cta.download": "Windows के लिए डाउनलोड करें", "cta.github": "GitHub पर देखें",
    "footer.product": "उत्पाद", "footer.changelog": "बदलाव", "footer.community": "समुदाय", "footer.releases": "रिलीज़",
    "footer.resources": "संसाधन", "footer.docs": "दस्तावेज़ीकरण", "footer.install": "स्थापना", "footer.shortcuts": "शॉर्टकट",
    "footer.legal": "कानूनी", "footer.license": "MIT लाइसेंस", "footer.social": "सोशल", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windows फ़ॉर्क:", "plugin.badge": "Claude Code प्लगइन", "plugin.desc": "एक ही कमांड से कई Claude Code एजेंट समानांतर में लॉन्च करें। प्लगइन आपके कोडबेस का विश्लेषण करता है, कार्य को स्वतंत्र उप-कार्यों में विभाजित करता है और उन्हें निर्भरता प्रबंधन, पृथक फ़ाइल ज़ोन और प्रत्येक चक्र के अंत में स्वचालित समीक्षा के साथ तरंगों में वितरित करता है।", "plugin.install_label": "इंस्टॉल", "plugin.view_github": "GitHub पर देखें", "plugin.docs": "प्रलेखन",
  },

  // ═══════════════════════════ VIETNAMESE ═══════════════════════════
  vi: {
    "nav.docs": "Tài liệu", "nav.changelog": "Nhật ký thay đổi", "nav.community": "Cộng đồng", "nav.github": "GitHub",
    "header.download": "Tải cho Windows",
    "hero.tagline": "Terminal được thiết kế cho", "hero.word1": "Claude Code", "hero.word2": "AI agents", "hero.word3": "đa nhiệm", "hero.word4": "Windows",
    "hero.desc": "Ứng dụng Windows gốc được viết bằng Rust. Tab dọc, thông báo khi agent cần chú ý, bảng chia, phiên SSH từ xa và API pipe để tự động hóa.",
    "hero.download": "Tải cho Windows", "hero.github": "Xem trên GitHub",
    "features.title": "Tính năng",
    "f1.name": "Tích hợp thụ động Claude Code", "f1.desc": "PandaMUX quan sát Claude Code mà không thay đổi hành vi. Các hook được cấu hình tự động báo cáo hoạt động của agent và công cụ lên thanh bên.",
    "f2.name": "Vòng thông báo", "f2.desc": "bảng phát sáng xanh khi agent cần chú ý. Thông báo toast Windows.",
    "f3.name": "Tab dọc", "f3.desc": "thanh bên hiển thị nhánh git, thư mục làm việc, cổng, số agent, trạng thái PR.",
    "f4.name": "Bảng chia", "f4.desc": "chia ngang và dọc trong mỗi tab.",
    "f5.name": "Chỉ báo hoạt động", "f5.desc": "cam nhấp nháy = đang làm, xanh = xong, đỏ = bị gián đoạn.",
    "f6.name": "Phiên đã lưu", "f6.desc": "lưu bố cục chia và thư mục. Tự động khôi phục khi khởi động.",
    "f7.name": "Có thể lập trình", "f7.desc": 'CLI và API named pipe (<code>\\\\.\\pipe\\pandamux</code>) để tự động hóa.',
    "f8.name": "Windows gốc", "f8.desc": "ConPTY, thông báo toast Windows, nhấp nháy thanh tác vụ.",
    "f9.name": "Tăng tốc GPU", "f9.desc": "được cung cấp bởi wgpu với GPU rendering.",
    "f10.name": "Tương thích chủ đề", "f10.desc": "nhập chủ đề từ Windows Terminal hoặc Ghostty. 450+ chủ đề Ghostty đi kèm.",
    "f11.name": "Phím tắt", "f11.desc": "phím tắt đầy đủ cho workspace, chia, phiên SSH và hơn thế nữa.",
    "faq.title": "Câu hỏi thường gặp",
    "faq1.q": "Mối quan hệ giữa PandaMUX và cmux?", "faq1.a": 'PandaMUX là fork Windows của <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>.',
    "faq2.q": "Hỗ trợ nền tảng nào?", "faq2.a": 'Chỉ Windows 10 và 11 (x64). Cho macOS dùng <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Agent nào tương thích?", "faq3.a": "PandaMUX được tối ưu cho Claude Code. Bất kỳ agent dòng lệnh nào đều có thể sử dụng.",
    "faq4.q": "Thông báo hoạt động như thế nào?", "faq4.a": 'Script tích hợp shell phát hiện lệnh hoàn thành hoặc bị gián đoạn. Hỗ trợ OSC 9/99/777 và <code>pandamux notify</code>.',
    "faq5.q": "Khác gì với Windows Terminal?", "faq5.a": "PandaMUX được thiết kế đặc biệt để giám sát AI agent.",
    "faq6.q": "Miễn phí không?", "faq6.a": 'Có. PandaMUX là mã nguồn mở theo giấy phép MIT. <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Tải cho Windows", "cta.github": "Xem trên GitHub",
    "footer.product": "Sản phẩm", "footer.changelog": "Nhật ký", "footer.community": "Cộng đồng", "footer.releases": "Phát hành",
    "footer.resources": "Tài nguyên", "footer.docs": "Tài liệu", "footer.install": "Cài đặt", "footer.shortcuts": "Phím tắt",
    "footer.legal": "Pháp lý", "footer.license": "Giấy phép MIT", "footer.social": "Mạng xã hội", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Fork Windows của", "plugin.badge": "Plugin Claude Code", "plugin.desc": "Chạy nhiều agent Claude Code song song chỉ với một lệnh. Plugin phân tích mã nguồn, chia tác vụ thành các tác vụ con độc lập và phân phối chúng theo đợt với quản lý phụ thuộc, vùng tệp cô lập và đánh giá tự động cuối mỗi chu kỳ.", "plugin.install_label": "Cài đặt", "plugin.view_github": "Xem trên GitHub", "plugin.docs": "Tài liệu",
  },

  // ═══════════════════════════ THAI ═══════════════════════════
  th: {
    "nav.docs": "เอกสาร", "nav.changelog": "บันทึกการเปลี่ยนแปลง", "nav.community": "ชุมชน", "nav.github": "GitHub",
    "header.download": "ดาวน์โหลดสำหรับ Windows",
    "hero.tagline": "เทอร์มินัลที่สร้างสำหรับ", "hero.word1": "Claude Code", "hero.word2": "AI agents", "hero.word3": "มัลติทาสกิ้ง", "hero.word4": "Windows",
    "hero.desc": "แอปพลิเคชัน Windows เนทีฟที่เขียนด้วย Rust แท็บแนวตั้ง การแจ้งเตือนเมื่อ agent ต้องการความสนใจ แผงแบ่ง เซสชัน SSH ระยะไกล และ API ท่อสำหรับอัตโนมัติ",
    "hero.download": "ดาวน์โหลดสำหรับ Windows", "hero.github": "ดูบน GitHub",
    "features.title": "คุณสมบัติ",
    "f1.name": "การรวม Claude Code แบบพาสซีฟ", "f1.desc": "PandaMUX สังเกต Claude Code โดยไม่เปลี่ยนแปลงพฤติกรรม ฮุกที่กำหนดค่าอัตโนมัติรายงานกิจกรรมของ agent และเครื่องมือไปยังแถบด้านข้าง",
    "f2.name": "วงแหวนแจ้งเตือน", "f2.desc": "แผงเรืองแสงสีน้ำเงินเมื่อ agent ต้องการความสนใจ",
    "f3.name": "แท็บแนวตั้ง", "f3.desc": "แถบด้านข้างแสดง git branch ไดเรกทอรีทำงาน พอร์ต จำนวน agent",
    "f4.name": "แผงแบ่ง", "f4.desc": "แบ่งแนวนอนและแนวตั้งในแต่ละแท็บ",
    "f5.name": "ตัวบ่งชี้กิจกรรม", "f5.desc": "ส้มกระพริบ = ทำงาน เขียว = เสร็จ แดง = ถูกขัดจังหวะ",
    "f6.name": "เซสชันที่บันทึก", "f6.desc": "บันทึกการแบ่งและไดเรกทอรี กู้คืนอัตโนมัติเมื่อเริ่มต้น",
    "f7.name": "เขียนสคริปต์ได้", "f7.desc": 'CLI และ API named pipe (<code>\\\\.\\pipe\\pandamux</code>) สำหรับอัตโนมัติ',
    "f8.name": "Windows เนทีฟ", "f8.desc": "ConPTY การแจ้งเตือน toast ของ Windows",
    "f9.name": "GPU acceleration", "f9.desc": "ขับเคลื่อนโดย wgpu พร้อม GPU rendering",
    "f10.name": "ธีมที่เข้ากันได้", "f10.desc": "นำเข้าธีมจาก Windows Terminal หรือ Ghostty 450+ ธีม Ghostty รวมอยู่",
    "f11.name": "ปุ่มลัด", "f11.desc": "ปุ่มลัดครบถ้วนสำหรับเวิร์กสเปซ การแบ่ง เซสชัน SSH และอื่นๆ",
    "faq.title": "คำถามที่พบบ่อย",
    "faq1.q": "PandaMUX และ cmux เกี่ยวข้องกันอย่างไร?", "faq1.a": 'PandaMUX เป็น fork Windows ของ <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>',
    "faq2.q": "รองรับแพลตฟอร์มใดบ้าง?", "faq2.a": 'Windows 10 และ 11 เท่านั้น (x64) สำหรับ macOS ใช้ <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>',
    "faq3.q": "agent ใดที่เข้ากันได้?", "faq3.a": "PandaMUX ปรับให้เหมาะสมสำหรับ Claude Code agent บรรทัดคำสั่งใดก็ได้สามารถใช้ได้",
    "faq4.q": "การแจ้งเตือนทำงานอย่างไร?", "faq4.a": 'สคริปต์รวม shell ตรวจจับเมื่อคำสั่งเสร็จสิ้นหรือถูกขัดจังหวะ รองรับ OSC 9/99/777 และ <code>pandamux notify</code>',
    "faq5.q": "แตกต่างจาก Windows Terminal อย่างไร?", "faq5.a": "PandaMUX ออกแบบมาเพื่อดูแล AI agent โดยเฉพาะ",
    "faq6.q": "ฟรีหรือไม่?", "faq6.a": 'ใช่ PandaMUX เป็นโอเพนซอร์สภายใต้ MIT <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>',
    "cta.download": "ดาวน์โหลดสำหรับ Windows", "cta.github": "ดูบน GitHub",
    "footer.product": "ผลิตภัณฑ์", "footer.changelog": "บันทึก", "footer.community": "ชุมชน", "footer.releases": "รุ่น",
    "footer.resources": "ทรัพยากร", "footer.docs": "เอกสาร", "footer.install": "การติดตั้ง", "footer.shortcuts": "ปุ่มลัด",
    "footer.legal": "กฎหมาย", "footer.license": "สัญญาอนุญาต MIT", "footer.social": "โซเชียล", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Fork Windows ของ", "plugin.badge": "ปลั๊กอิน Claude Code", "plugin.desc": "เรียกใช้ตัวแทน Claude Code หลายตัวพร้อมกันด้วยคำสั่งเดียว ปลั๊กอินวิเคราะห์โค้ดเบส แบ่งงานเป็นงานย่อยอิสระ และกระจายเป็นคลื่นพร้อมจัดการการพึ่งพา โซนไฟล์แยก และตรวจสอบอัตโนมัติเมื่อสิ้นสุดรอบ", "plugin.install_label": "ติดตั้ง", "plugin.view_github": "ดูบน GitHub", "plugin.docs": "เอกสาร",
  },

  // ═══════════════════════════ INDONESIAN ═══════════════════════════
  id: {
    "nav.docs": "Dokumentasi", "nav.changelog": "Perubahan", "nav.community": "Komunitas", "nav.github": "GitHub",
    "header.download": "Unduh untuk Windows",
    "hero.tagline": "Terminal yang dibuat untuk", "hero.word1": "Claude Code", "hero.word2": "agen AI", "hero.word3": "multitasking", "hero.word4": "Windows",
    "hero.desc": "Aplikasi Windows native yang ditulis dengan Rust. Tab vertikal, notifikasi saat agen butuh perhatian, panel terbagi, sesi SSH jarak jauh dan API pipe untuk otomatisasi.",
    "hero.download": "Unduh untuk Windows", "hero.github": "Lihat di GitHub",
    "features.title": "Fitur",
    "f1.name": "Integrasi pasif Claude Code", "f1.desc": "PandaMUX mengamati Claude Code tanpa mengubah perilakunya. Hook yang dikonfigurasi otomatis melaporkan aktivitas agen dan alat ke bilah sisi.",
    "f2.name": "Cincin notifikasi", "f2.desc": "panel bersinar biru saat agen butuh perhatian. Notifikasi toast Windows.",
    "f3.name": "Tab vertikal", "f3.desc": "sidebar menampilkan branch git, direktori kerja, port, jumlah agen, status PR.",
    "f4.name": "Panel terbagi", "f4.desc": "pembagian horizontal dan vertikal di setiap tab.",
    "f5.name": "Indikator aktivitas", "f5.desc": "oranye berkedip = bekerja, hijau = selesai, merah = terganggu.",
    "f6.name": "Sesi tersimpan", "f6.desc": "simpan pembagian dan direktori. Pemulihan otomatis saat startup.",
    "f7.name": "Dapat diprogram", "f7.desc": 'CLI dan API named pipe (<code>\\\\.\\pipe\\pandamux</code>) untuk otomatisasi.',
    "f8.name": "Windows native", "f8.desc": "ConPTY, notifikasi toast Windows, kedipan taskbar.",
    "f9.name": "Akselerasi GPU", "f9.desc": "didukung wgpu dengan rendering GPU.",
    "f10.name": "Kompatibel tema", "f10.desc": "impor tema dari Windows Terminal atau Ghostty. 450+ tema Ghostty disertakan.",
    "f11.name": "Pintasan keyboard", "f11.desc": "pintasan lengkap untuk workspace, pembagian, sesi SSH dan lainnya.",
    "faq.title": "Pertanyaan yang sering diajukan",
    "faq1.q": "Apa hubungan PandaMUX dan cmux?", "faq1.a": 'PandaMUX adalah fork Windows dari <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>.',
    "faq2.q": "Platform apa yang didukung?", "faq2.a": 'Hanya Windows 10 dan 11 (x64). Untuk macOS gunakan <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Agen apa yang kompatibel?", "faq3.a": "PandaMUX dioptimalkan untuk Claude Code. Agen baris perintah apa pun bisa digunakan.",
    "faq4.q": "Bagaimana notifikasi bekerja?", "faq4.a": 'Skrip integrasi shell mendeteksi perintah selesai atau terganggu. Mendukung OSC 9/99/777 dan <code>pandamux notify</code>.',
    "faq5.q": "Apa bedanya dengan Windows Terminal?", "faq5.a": "PandaMUX dirancang khusus untuk mengawasi agen AI.",
    "faq6.q": "Gratis?", "faq6.a": 'Ya. PandaMUX adalah open-source di bawah lisensi MIT. <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Unduh untuk Windows", "cta.github": "Lihat di GitHub",
    "footer.product": "Produk", "footer.changelog": "Perubahan", "footer.community": "Komunitas", "footer.releases": "Rilis",
    "footer.resources": "Sumber daya", "footer.docs": "Dokumentasi", "footer.install": "Instalasi", "footer.shortcuts": "Pintasan",
    "footer.legal": "Hukum", "footer.license": "Lisensi MIT", "footer.social": "Sosial", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Fork Windows dari", "plugin.badge": "Plugin Claude Code", "plugin.desc": "Jalankan beberapa agen Claude Code secara paralel dengan satu perintah. Plugin menganalisis basis kode, membagi tugas menjadi sub-tugas independen, dan mendistribusikannya dalam gelombang dengan manajemen dependensi, zona file terisolasi, dan tinjauan otomatis di akhir setiap siklus.", "plugin.install_label": "Instalasi", "plugin.view_github": "Lihat di GitHub", "plugin.docs": "Dokumentasi",
  },

  // ═══════════════════════════ SWEDISH ═══════════════════════════
  sv: {
    "nav.docs": "Docs", "nav.changelog": "Ändringar", "nav.community": "Community", "nav.github": "GitHub",
    "header.download": "Ladda ner för Windows",
    "hero.tagline": "Terminalen byggd för", "hero.word1": "Claude Code", "hero.word2": "AI-agenter", "hero.word3": "multitasking", "hero.word4": "Windows",
    "hero.desc": "Inbyggd Windows-applikation skriven i Rust. Vertikala flikar, aviseringar när agenter behöver uppmärksamhet, delade paneler, SSH-fjärrsessioner och ett pipe-API för automatisering.",
    "hero.download": "Ladda ner för Windows", "hero.github": "Visa på GitHub",
    "features.title": "Funktioner",
    "f1.name": "Passiv Claude Code-integration", "f1.desc": "PandaMUX observerar Claude Code utan att ändra dess beteende. Automatiskt konfigurerade hooks rapporterar agent- och verktygsaktivitet i sidofältet.",
    "f2.name": "Aviseringsringar", "f2.desc": "paneler lyser blått när agenter behöver uppmärksamhet. Windows toast-aviseringar.",
    "f3.name": "Vertikala flikar", "f3.desc": "sidofältet visar git-gren, arbetskatalog, portar, antal agenter, PR-status.",
    "f4.name": "Delade paneler", "f4.desc": "horisontella och vertikala delningar i varje flik.",
    "f5.name": "Aktivitetsindikatorer", "f5.desc": "pulserande orange = arbetar, grönt = klart, rött = avbrutet.",
    "f6.name": "Sparade sessioner", "f6.desc": "spara dina delningar och kataloger. Automatisk återställning vid start.",
    "f7.name": "Skriptbar", "f7.desc": 'CLI och named pipe API (<code>\\\\.\\pipe\\pandamux</code>) för automatisering.',
    "f8.name": "Windows-nativ", "f8.desc": "ConPTY, Windows toast-aviseringar, aktivitetsfältsblink.",
    "f9.name": "GPU-acceleration", "f9.desc": "drivs av wgpu med GPU-rendering.",
    "f10.name": "Temakompatibel", "f10.desc": "importera teman från Windows Terminal eller Ghostty. 450+ Ghostty-teman inkluderade.",
    "f11.name": "Tangentbordsgenvägar", "f11.desc": "fullständiga genvägar för arbetsytor, delningar, SSH-sessioner och mer.",
    "faq.title": "Vanliga frågor",
    "faq1.q": "Vad är förhållandet mellan PandaMUX och cmux?", "faq1.a": 'PandaMUX är en Windows-fork av <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>.',
    "faq2.q": "Vilka plattformar stöds?", "faq2.a": 'Bara Windows 10 och 11 (x64). För macOS använd <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Vilka agenter är kompatibla?", "faq3.a": "PandaMUX är optimerad för Claude Code. Alla kommandoradsagenter kan användas.",
    "faq4.q": "Hur fungerar aviseringarna?", "faq4.a": 'Shell-integrationsskript upptäcker när ett kommando avslutas eller avbryts. Stöder OSC 9/99/777 och <code>pandamux notify</code>.',
    "faq5.q": "Hur skiljer det sig från Windows Terminal?", "faq5.a": "PandaMUX är specifikt designad för att övervaka AI-agenter.",
    "faq6.q": "Är det gratis?", "faq6.a": 'Ja. PandaMUX är öppen källkod under MIT. <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Ladda ner för Windows", "cta.github": "Visa på GitHub",
    "footer.product": "Produkt", "footer.changelog": "Ändringar", "footer.community": "Community", "footer.releases": "Utgåvor",
    "footer.resources": "Resurser", "footer.docs": "Dokumentation", "footer.install": "Installation", "footer.shortcuts": "Genvägar",
    "footer.legal": "Juridiskt", "footer.license": "MIT-licens", "footer.social": "Socialt", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windows-fork av", "plugin.badge": "Claude Code-plugin", "plugin.desc": "Starta flera Claude Code-agenter parallellt med ett enda kommando. Pluginet analyserar din kodbas, delar upp uppgiften i oberoende deluppgifter och fördelar dem i vågor med beroendehantering, isolerade filzoner och automatisk granskning i slutet av varje cykel.", "plugin.install_label": "Installation", "plugin.view_github": "Visa på GitHub", "plugin.docs": "Dokumentation",
  },

  // ═══════════════════════════ CZECH ═══════════════════════════
  cs: {
    "nav.docs": "Dokumentace", "nav.changelog": "Změny", "nav.community": "Komunita", "nav.github": "GitHub",
    "header.download": "Stáhnout pro Windows",
    "hero.tagline": "Terminál navržený pro", "hero.word1": "Claude Code", "hero.word2": "AI agenty", "hero.word3": "multitasking", "hero.word4": "Windows",
    "hero.desc": "Nativní Windows aplikace napsaná v Rustu. Vertikální karty, upozornění když agenti potřebují pozornost, rozdělené panely, vzdálené relace SSH a pipe API pro automatizaci.",
    "hero.download": "Stáhnout pro Windows", "hero.github": "Zobrazit na GitHubu",
    "features.title": "Funkce",
    "f1.name": "Pasivní integrace Claude Code", "f1.desc": "PandaMUX pozoruje Claude Code bez změny jeho chování. Automaticky konfigurované hooky hlásí aktivitu agentů a nástrojů v postranním panelu.",
    "f2.name": "Oznamovací kroužky", "f2.desc": "panely svítí modře když agenti potřebují pozornost. Windows toast upozornění.",
    "f3.name": "Vertikální karty", "f3.desc": "boční panel zobrazuje git větev, pracovní adresář, porty, počet agentů, stav PR.",
    "f4.name": "Rozdělené panely", "f4.desc": "horizontální a vertikální rozdělení v každé kartě.",
    "f5.name": "Ukazatele aktivity", "f5.desc": "pulzující oranžová = pracuje, zelená = hotovo, červená = přerušeno.",
    "f6.name": "Uložené relace", "f6.desc": "uložte rozdělení a adresáře. Automatické obnovení při spuštění.",
    "f7.name": "Skriptovatelný", "f7.desc": 'CLI a named pipe API (<code>\\\\.\\pipe\\pandamux</code>) pro automatizaci.',
    "f8.name": "Windows nativní", "f8.desc": "ConPTY, Windows toast upozornění, blikání hlavního panelu.",
    "f9.name": "GPU akcelerace", "f9.desc": "poháněno wgpu s GPU renderováním.",
    "f10.name": "Kompatibilní témata", "f10.desc": "importujte témata z Windows Terminal nebo Ghostty. 450+ Ghostty témat v balení.",
    "f11.name": "Klávesové zkratky", "f11.desc": "kompletní zkratky pro pracovní prostory, rozdělení, relace SSH a další.",
    "faq.title": "Často kladené otázky",
    "faq1.q": "Jaký je vztah mezi PandaMUX a cmux?", "faq1.a": 'PandaMUX je Windows fork projektu <a href="https://github.com/manaflow-ai/cmux" target="_blank" rel="noopener">cmux</a>.',
    "faq2.q": "Jaké platformy jsou podporovány?", "faq2.a": 'Pouze Windows 10 a 11 (x64). Pro macOS použijte <a href="https://cmux.com" target="_blank" rel="noopener">cmux</a>.',
    "faq3.q": "Kteří agenti jsou kompatibilní?", "faq3.a": "PandaMUX je optimalizován pro Claude Code. Jakýkoli agent příkazové řádky může být použit.",
    "faq4.q": "Jak fungují upozornění?", "faq4.a": 'Skripty integrace shellu detekují dokončení nebo přerušení příkazu. Podporuje OSC 9/99/777 a <code>pandamux notify</code>.',
    "faq5.q": "Čím se liší od Windows Terminal?", "faq5.a": "PandaMUX je navržen speciálně pro dohled nad AI agenty.",
    "faq6.q": "Je to zdarma?", "faq6.a": 'Ano. PandaMUX je open-source pod licencí MIT. <a href="https://github.com/BoardPandas/Pandamux/releases" target="_blank" rel="noopener">GitHub Releases</a>.',
    "cta.download": "Stáhnout pro Windows", "cta.github": "Zobrazit na GitHubu",
    "footer.product": "Produkt", "footer.changelog": "Změny", "footer.community": "Komunita", "footer.releases": "Vydání",
    "footer.resources": "Zdroje", "footer.docs": "Dokumentace", "footer.install": "Instalace", "footer.shortcuts": "Zkratky",
    "footer.legal": "Právní", "footer.license": "Licence MIT", "footer.social": "Sociální sítě", "footer.copyright": "© 2026 PandaMUX", "footer.fork": "Windows fork projektu", "plugin.badge": "Plugin Claude Code", "plugin.desc": "Spusťte více agentů Claude Code paralelně jedním příkazem. Plugin analyzuje kódovou základnu, rozdělí úkol na nezávislé podúkoly a distribuuje je ve vlnách se správou závislostí, izolovanými souborovými zónami a automatickou kontrolou na konci každého cyklu.", "plugin.install_label": "Instalace", "plugin.view_github": "Zobrazit na GitHubu", "plugin.docs": "Dokumentace",
  },
};

// ═══════════════════════════ ENGINE ═══════════════════════════

function detectLang() {
  // 1. URL hash (#fr, #ar, etc.)
  const hash = location.hash.replace("#", "").toLowerCase();
  if (hash && T[hash]) return hash;
  // Also try zh-TW style
  if (hash === "zh-tw" && T["zh-TW"]) return "zh-TW";

  // 2. localStorage preference
  const saved = localStorage.getItem("pandamux-lang");
  if (saved && T[saved]) return saved;

  // 3. Browser language
  const nav = navigator.language || navigator.userLanguage || "en";
  // Try exact match first (e.g., zh-TW)
  if (T[nav]) return nav;
  // Try base language (e.g., "fr-FR" → "fr")
  const base = nav.split("-")[0].toLowerCase();
  if (T[base]) return base;
  // Special: zh-Hans → zh, zh-Hant → zh-TW
  if (base === "zh") {
    if (nav.includes("Hant") || nav.includes("TW") || nav.includes("HK")) return "zh-TW";
    return "zh";
  }

  return "en";
}

function applyLang(lang) {
  const strings = T[lang];
  if (!strings) return;

  // Set direction
  const isRTL = RTL_LANGS.includes(lang);
  document.documentElement.dir = isRTL ? "rtl" : "ltr";
  document.documentElement.lang = lang;

  // Update all data-i18n elements
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    if (strings[key] != null) {
      el.textContent = strings[key];
    }
  });

  // Update all data-i18n-html elements (contain HTML like <code>, <a>)
  document.querySelectorAll("[data-i18n-html]").forEach((el) => {
    const key = el.getAttribute("data-i18n-html");
    if (strings[key] != null) {
      el.innerHTML = strings[key];
    }
  });

  // Update page title
  document.title = `PandaMUX — ${strings["hero.tagline"]} Claude Code`;

  // Update language selector display
  const sel = document.getElementById("lang-current");
  if (sel) sel.textContent = LANGS[lang] || lang;

  // Save preference
  localStorage.setItem("pandamux-lang", lang);

  // Update hash without scrolling
  history.replaceState(null, "", `#${lang}`);
}

function buildLangSelector() {
  const container = document.getElementById("lang-selector");
  if (!container) return;

  const dropdown = document.createElement("div");
  dropdown.className = "lang-dropdown";

  Object.entries(LANGS).forEach(([code, name]) => {
    const btn = document.createElement("button");
    btn.textContent = name;
    btn.className = "lang-option";
    btn.setAttribute("data-lang", code);
    btn.addEventListener("click", () => {
      applyLang(code);
      dropdown.classList.remove("open");
    });
    dropdown.appendChild(btn);
  });

  container.appendChild(dropdown);

  const trigger = container.querySelector(".lang-trigger");
  if (trigger) {
    trigger.addEventListener("click", (e) => {
      e.stopPropagation();
      dropdown.classList.toggle("open");
    });
  }

  // Close on outside click
  document.addEventListener("click", () => dropdown.classList.remove("open"));
}

// Init — script is at bottom of <body>, DOM is already parsed
function init() {
  buildLangSelector();
  const lang = detectLang();
  applyLang(lang);
}

// Use requestAnimationFrame to ensure paint has happened and DOM is fully wired
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", () => requestAnimationFrame(init));
} else {
  requestAnimationFrame(init);
}

// Also handle hash changes without page reload
window.addEventListener("hashchange", () => {
  const hash = location.hash.replace("#", "").toLowerCase();
  const lang = hash === "zh-tw" ? "zh-TW" : hash;
  if (T[lang]) applyLang(lang);
});
