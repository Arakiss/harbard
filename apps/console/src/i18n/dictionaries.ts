import enWorkspace from "./dictionaries/en/workspace.json";
import enMessages from "./dictionaries/en/messages.json";
import enAttachments from "./dictionaries/en/attachments.json";
import enAgents from "./dictionaries/en/agents.json";
import enShell from "./dictionaries/en/shell.json";
import enHome from "./dictionaries/en/home.json";
import enPreferences from "./dictionaries/en/preferences.json";
import enProfile from "./dictionaries/en/profile.json";
import enErrors from "./dictionaries/en/errors.json";
import enAvatars from "./dictionaries/en/avatars.json";
import enDesktop from "./dictionaries/en/desktop.json";
import enUpdate from "./dictionaries/en/update.json";
import enTeams from "./dictionaries/en/teams.json";
import enSetup from "./dictionaries/en/setup.json";
import enDeliveries from "./dictionaries/en/deliveries.json";
import enProgress from "./dictionaries/en/progress.json";
import enActivity from "./dictionaries/en/activity.json";
import enDirect from "./dictionaries/en/direct.json";
import enHistory from "./dictionaries/en/history.json";
import enLater from "./dictionaries/en/later.json";
import enRetro from "./dictionaries/en/retro.json";
import enFeedback from "./dictionaries/en/feedback.json";
import enStrip from "./dictionaries/en/strip.json";
import enHuginn from "./dictionaries/en/huginn.json";
import enServe from "./dictionaries/en/serve.json";
import esWorkspace from "./dictionaries/es/workspace.json";
import esMessages from "./dictionaries/es/messages.json";
import esAttachments from "./dictionaries/es/attachments.json";
import esAgents from "./dictionaries/es/agents.json";
import esShell from "./dictionaries/es/shell.json";
import esHome from "./dictionaries/es/home.json";
import esPreferences from "./dictionaries/es/preferences.json";
import esProfile from "./dictionaries/es/profile.json";
import esErrors from "./dictionaries/es/errors.json";
import esAvatars from "./dictionaries/es/avatars.json";
import esDesktop from "./dictionaries/es/desktop.json";
import esUpdate from "./dictionaries/es/update.json";
import esTeams from "./dictionaries/es/teams.json";
import esSetup from "./dictionaries/es/setup.json";
import esDeliveries from "./dictionaries/es/deliveries.json";
import esProgress from "./dictionaries/es/progress.json";
import esActivity from "./dictionaries/es/activity.json";
import esDirect from "./dictionaries/es/direct.json";
import esHistory from "./dictionaries/es/history.json";
import esLater from "./dictionaries/es/later.json";
import esRetro from "./dictionaries/es/retro.json";
import esFeedback from "./dictionaries/es/feedback.json";
import esStrip from "./dictionaries/es/strip.json";
import esHuginn from "./dictionaries/es/huginn.json";
import esServe from "./dictionaries/es/serve.json";

/**
 * English is the reference: its JSON files define which keys exist. Each
 * Spanish file must have the same keys, and the annotation on `es` turns a
 * missing one into a compile error.
 */
export const en = {
  workspace: enWorkspace,
  messages: enMessages,
  attachments: enAttachments,
  agents: enAgents,
  shell: enShell,
  home: enHome,
  preferences: enPreferences,
  profile: enProfile,
  errors: enErrors,
  avatars: enAvatars,
  desktop: enDesktop,
  update: enUpdate,
  teams: enTeams,
  setup: enSetup,
  deliveries: enDeliveries,
  progress: enProgress,
  activity: enActivity,
  direct: enDirect,
  history: enHistory,
  later: enLater,
  retro: enRetro,
  feedback: enFeedback,
  strip: enStrip,
  huginn: enHuginn,
  serve: enServe,
};

export type Feature = keyof typeof en;
export type Dictionaries = typeof en;

type SameKeys<Dictionary> = { [Key in keyof Dictionary]: string };

export const es: { [F in Feature]: SameKeys<Dictionaries[F]> } = {
  workspace: esWorkspace,
  messages: esMessages,
  attachments: esAttachments,
  agents: esAgents,
  shell: esShell,
  home: esHome,
  preferences: esPreferences,
  profile: esProfile,
  errors: esErrors,
  avatars: esAvatars,
  desktop: esDesktop,
  update: esUpdate,
  teams: esTeams,
  setup: esSetup,
  deliveries: esDeliveries,
  progress: esProgress,
  activity: esActivity,
  direct: esDirect,
  history: esHistory,
  later: esLater,
  retro: esRetro,
  feedback: esFeedback,
  strip: esStrip,
  huginn: esHuginn,
  serve: esServe,
};
