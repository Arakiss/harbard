/**
 * serve's fixed sentences, in the operator's language. serve sends each one
 * in English beside a stable code and its params; a code this build does not
 * know shows serve's English.
 */
import { type TranslationKey, type TranslationVars, createT } from "@/i18n";

const t = createT("serve");

export type ServeParams = TranslationVars;

const SERVE_TEXT: Record<string, TranslationKey<"serve">> = {
  "theme.bad_name.invalid": "themeBadNameInvalid",
  "theme.bad_name.invalid.fix": "themeBadNameInvalidFix",
  "theme.bad_name.builtin": "themeBadNameBuiltin",
  "theme.bad_name.builtin.fix": "themeBadNameBuiltinFix",
  "theme.bad_name.fallback": "themeBadNameFallback",
  "theme.bad_name.fallback.fix": "themeBadNameFallbackFix",
  "theme.bad_colour.alpha": "themeBadColourAlpha",
  "theme.bad_colour.alpha.fix": "themeBadColourAlphaFix",
  "theme.bad_colour.unquoted": "themeBadColourUnquoted",
  "theme.bad_colour.unquoted.fix": "themeBadColourUnquotedFix",
  "theme.bad_colour.hex": "themeBadColourHex",
  "theme.bad_colour.hex.fix": "themeBadColourHexFix",
  "theme.duplicate_key.dark_table": "themeDuplicateKeyDarkTable",
  "theme.duplicate_key.dark_table.fix": "themeDuplicateKeyDarkTableFix",
  "theme.duplicate_key.key": "themeDuplicateKeyKey",
  "theme.duplicate_key.key.fix": "themeDuplicateKeyKeyFix",
  "theme.unknown_section": "themeUnknownSection",
  "theme.unknown_section.fix": "themeUnknownSectionFix",
  "theme.syntax.line": "themeSyntaxLine",
  "theme.syntax.line.fix": "themeSyntaxLineFix",
  "theme.syntax.trailing": "themeSyntaxTrailing",
  "theme.syntax.trailing.fix": "themeSyntaxTrailingFix",
  "theme.unknown_key": "themeUnknownKey",
  "theme.unknown_key.fix": "themeUnknownKeyFix",
  "theme.unknown_key.fix_suggest": "themeUnknownKeyFixSuggest",
  "theme.missing_key": "themeMissingKey",
  "theme.missing_key.fix": "themeMissingKeyFix",
  "theme.read_failed.read": "themeReadFailedRead",
  "theme.read_failed.path.fix": "themeReadFailedPathFix",
  "theme.read_failed.encoding.fix": "themeReadFailedEncodingFix",
  "theme.read_failed.not_a_file": "themeReadFailedNotAFile",
  "theme.read_failed.not_a_file.fix": "themeReadFailedNotAFileFix",
  "theme.read_failed.write": "themeReadFailedWrite",
  "theme.read_failed.write.fix": "themeReadFailedWriteFix",
  "theme.linked_dir": "themeLinkedDir",
  "theme.linked_dir.fix": "themeLinkedDirFix",
  "theme.missing_theme": "themeMissingTheme",
  "theme.missing_theme.fix": "themeMissingThemeFix",
  "theme.missing_theme.fix_case": "themeMissingThemeFixCase",
  "theme.bad_code.prefix": "themeBadCodePrefix",
  "theme.bad_code.name": "themeBadCodeName",
  "theme.bad_code.count": "themeBadCodeCount",
  "theme.bad_code.colour": "themeBadCodeColour",
  "theme.bad_code.alpha": "themeBadCodeAlpha",
  "theme.bad_code.fix": "themeBadCodeFix",
  "theme.exists": "themeExists",
  "theme.exists.fix": "themeExistsFix",
  "theme.bad_mode": "themeBadMode",
  "theme.bad_mode.fix": "themeBadModeFix",
  "theme.low_contrast.light": "themeLowContrastLight",
  "theme.low_contrast.dark": "themeLowContrastDark",
  "theme.low_contrast.fix_fg": "themeLowContrastFixFg",
  "theme.low_contrast.fix_bg": "themeLowContrastFixBg",
  "theme.notice.bad_mode": "themeNoticeBadMode",
  "theme.notice.refused": "themeNoticeRefused",
  "theme.notice.absent": "themeNoticeAbsent",
  "retro.evidence.shared_security": "retroEvidenceSharedSecurity",
  "retro.evidence.review_repeat": "retroEvidenceReviewRepeat",
  "retro.evidence.stalls": "retroEvidenceStalls",
  "retro.evidence.unanswered": "retroEvidenceUnanswered",
  "retro.evidence.plain_messages": "retroEvidencePlainMessages",
  "retro.evidence.readable_names": "retroEvidenceReadableNames",
  "retro.evidence.background_wait.one": "retroEvidenceBackgroundWaitOne",
  "retro.evidence.background_wait.many": "retroEvidenceBackgroundWaitMany",
  "retro.evidence.needs_you_noise": "retroEvidenceNeedsYouNoise",
  "retro.evidence.delivery_quality": "retroEvidenceDeliveryQuality",
  "readable.blocked_bare": "readableBlockedBare",
  "readable.tests.count": "readableTestsCount",
  "readable.tests.criteria": "readableTestsCriteria",
  "name.team_role": "nameTeamRole",
  "name.session": "nameSession",
  "name.session.time": "nameSessionTime",
  "name.session.project": "nameSessionProject",
  "name.session.project_time": "nameSessionProjectTime",
  "name.session.id": "nameSessionId",
  "role_line": "roleLine",
  "role_line.solo": "roleLineSolo",
  "readable.blocked": "readableBlocked",
  "readable.blocked.many": "readableBlockedMany",
  "readable.changes": "readableChanges",
  "readable.changes.many": "readableChangesMany",
  "readable.changes_on": "readableChangesOn",
  "readable.changes_on.many": "readableChangesOnMany",
  "readable.changes_on.start": "readableChangesOnStart",
  "readable.changes_on.start.many": "readableChangesOnStartMany",
  "readable.approved": "readableApproved",
  "readable.approved.many": "readableApprovedMany",
  "readable.answered": "readableAnswered",
  "readable.answered.many": "readableAnsweredMany",
  "readable.fix_pushed": "readableFixPushed",
  "readable.fix_pushed.many": "readableFixPushedMany",
  "readable.merged": "readableMerged",
  "readable.merged.many": "readableMergedMany",
  "readable.pushed": "readablePushed",
  "readable.pushed.many": "readablePushedMany",
  "readable.green": "readableGreen",
  "readable.green.many": "readableGreenMany",
  "readable.fixed.pushed": "readableFixedPushed",
  "readable.fixed.pushed.many": "readableFixedPushedMany",
  "readable.fixed.unpushed": "readableFixedUnpushed",
  "readable.fixed.unpushed.many": "readableFixedUnpushedMany",
  "readable.fixed": "readableFixed",
  "readable.fixed.many": "readableFixedMany",
  "readable.done.pushed": "readableDonePushed",
  "readable.done.pushed.many": "readableDonePushedMany",
  "readable.done.unpushed": "readableDoneUnpushed",
  "readable.done.unpushed.many": "readableDoneUnpushedMany",
  "readable.done": "readableDone",
  "readable.done.many": "readableDoneMany",
  "readable.committed.pushed": "readableCommittedPushed",
  "readable.committed.pushed.many": "readableCommittedPushedMany",
  "readable.committed.unpushed": "readableCommittedUnpushed",
  "readable.committed.unpushed.many": "readableCommittedUnpushedMany",
  "readable.committed": "readableCommitted",
  "readable.committed.many": "readableCommittedMany",
};

/** The sentence for `code`, or `fallback` for a code this build does not know. */
export function serveText(code: string | null | undefined, params: ServeParams | null | undefined, fallback: string): string {
  const key = code ? SERVE_TEXT[code] : undefined;
  return key ? t(key, params ?? undefined) : fallback;
}

const PAIR_WHAT: Record<string, TranslationKey<"serve">> = {
  "rail_labels": "pairRailLabels",
  "active_rail_destination": "pairActiveRailDestination",
  "rail_button_icons": "pairRailButtonIcons",
  "rail_icons_on_hover": "pairRailIconsOnHover",
  "rail_logo": "pairRailLogo",
  "lit_rail_button": "pairLitRailButton",
  "unread_badge": "pairUnreadBadge",
  "sidebar_rows": "pairSidebarRows",
  "row_under_the_pointer": "pairRowUnderThePointer",
  "sections_and_counters": "pairSectionsAndCounters",
  "counter_on_the_row_under_the_pointer": "pairCounterOnTheRowUnderThePointer",
  "selected_channel": "pairSelectedChannel",
  "selected_channel_counter": "pairSelectedChannelCounter",
  "primary_button_and_counters": "pairPrimaryButtonAndCounters",
  "primary_button_under_the_pointer": "pairPrimaryButtonUnderThePointer",
  "pressed_button": "pairPressedButton",
  "active_tab": "pairActiveTab",
  "unread_pill": "pairUnreadPill",
  "accent_as_text": "pairAccentAsText",
  "links": "pairLinks",
  "links_in_lists": "pairLinksInLists",
  "channel_hash": "pairChannelHash",
  "hash_under_the_pointer": "pairHashUnderThePointer",
  "selected_channel_hash": "pairSelectedChannelHash",
  "green_dot_edge_on_the_sidebar": "pairGreenDotEdgeOnTheSidebar",
  "amber_dot_edge_on_the_sidebar": "pairAmberDotEdgeOnTheSidebar",
  "red_dot_edge_on_the_sidebar": "pairRedDotEdgeOnTheSidebar",
  "grey_dot_edge_on_the_sidebar": "pairGreyDotEdgeOnTheSidebar",
  "green_dot_edge_on_the_rail": "pairGreenDotEdgeOnTheRail",
  "amber_dot_edge_on_the_rail": "pairAmberDotEdgeOnTheRail",
  "red_dot_edge_on_the_rail": "pairRedDotEdgeOnTheRail",
  "grey_dot_edge_on_the_rail": "pairGreyDotEdgeOnTheRail",
  "text": "pairText",
  "secondary_text": "pairSecondaryText",
  "times_and_metadata": "pairTimesAndMetadata",
  "paths_and_hints": "pairPathsAndHints",
  "path_on_a_file_card": "pairPathOnAFileCard",
  "text_in_lists": "pairTextInLists",
  "preview_in_lists": "pairPreviewInLists",
  "times_in_lists": "pairTimesInLists",
  "menus_and_dialogs": "pairMenusAndDialogs",
  "secondary_text_in_dialogs": "pairSecondaryTextInDialogs",
  "selected_message": "pairSelectedMessage",
  "info_strip": "pairInfoStrip",
  "label": "pairLabel",
  "amber_label": "pairAmberLabel",
  "red_label": "pairRedLabel",
  "green_label": "pairGreenLabel",
  "protocol_notice": "pairProtocolNotice",
  "verified": "pairVerified",
  "warning_strip": "pairWarningStrip",
  "menu_note": "pairMenuNote",
  "error_banner": "pairErrorBanner",
  "error": "pairError",
  "send_button": "pairSendButton",
  "menu_option": "pairMenuOption",
  "warning_icon": "pairWarningIcon",
  "focus_ring": "pairFocusRing",
  "green_dot_edge": "pairGreenDotEdge",
  "amber_dot_edge": "pairAmberDotEdge",
  "red_dot_edge": "pairRedDotEdge",
  "grey_dot_edge": "pairGreyDotEdge",
};

/** What a contrast pair paints, by the key serve sends with a refusal. */
export function pairWhat(whatKey: string, fallback: string): string {
  const key = PAIR_WHAT[whatKey];
  return key ? t(key) : fallback;
}

/** Each role by its key: the masculine word, then the feminine one. */
const ROLE_WORD: Record<string, [TranslationKey<"serve">, TranslationKey<"serve">]> = {
  builder: ["roleBuilder", "roleBuilderF"],
  reviewer: ["roleReviewer", "roleReviewerF"],
  integration: ["roleIntegration", "roleIntegration"],
  lead: ["roleLead", "roleLeadF"],
  research: ["roleResearch", "roleResearch"],
  study: ["roleStudy", "roleStudy"],
  tests: ["roleTests", "roleTests"],
  fix: ["roleFix", "roleFix"],
  docs: ["roleDocs", "roleDocs"],
  design: ["roleDesign", "roleDesign"],
  audit: ["roleAudit", "roleAudit"],
  ops: ["roleOps", "roleOps"],
  eval: ["roleEval", "roleEval"],
  migration: ["roleMigration", "roleMigration"],
  upgrade: ["roleUpgrade", "roleUpgrade"],
  holdout: ["roleHoldout", "roleHoldout"],
  session: ["roleSession", "roleSession"],
};

function roleWord(role: string, gender: string | number | undefined): string {
  const keys = ROLE_WORD[role];
  if (!keys) return role;
  return t(gender === "f" ? keys[1] : keys[0]);
}

/** A coded role's params, its `role` replaced by the word to show. */
export function roleVars(params: ServeParams): ServeParams {
  const word = roleWord(String(params.role ?? ""), params.gender);
  return { ...params, role: params.number === undefined ? word : `${word} ${params.number}` };
}

/** A role as serve words it, `reviewer 2`, in the operator's language. */
export function localRole(role: string): string {
  const match = /^(\S+)( \d+)?$/.exec(role);
  if (!match?.[1] || !ROLE_WORD[match[1]]) return role;
  return `${roleWord(match[1], undefined)}${match[2] ?? ""}`;
}
