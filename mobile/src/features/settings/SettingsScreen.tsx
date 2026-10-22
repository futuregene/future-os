import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { Keyboard, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTranslation } from "react-i18next";
import { ArrowLeft } from "lucide-react-native";
import { LanguageSettings } from "../../i18n/LanguageSettings";
import { AppearanceSettings } from "../../theme/AppearanceSettings";
import { PEER_ICONS, iconGlyph } from "../../remote/peerIcons";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { DesktopSettings, RemoteBuiltinProvider, RemoteCustomProvider } from "../../remote/types";
import { VERSION } from "../../version.generated";
import { layout, radius, spacing } from "../../theme/tokens";
import { createThemedStyles, useTheme } from "../../theme/ThemeContext";
import { CustomProviderPage } from "./CustomProviderPage";
import { FollowAccountPage } from "./FollowAccountPage";
import { ModelsSettingsPage } from "./ModelsSettingsPage";
import { ProviderKeyPage } from "./ProviderKeyPage";
import { ProvidersSettingsPage } from "./ProvidersSettingsPage";
import { SkillsSettingsPage } from "./SkillsSettingsPage";
import { TasksSettingsPage } from "./TasksSettingsPage";
import type { SettingsPageBack } from "./pageBack";
import { ResourceStatus, SettingsField, SettingsLink, SettingsSection, SettingsSwitch, createSettingsStyles, useSettingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

export type SettingsScreenHandle = { goBack(): void };

const MAX_NAME_LENGTH = 40;

/**
 * Every settings level whose header names itself through `desktopSettings.<name>`.
 *
 * A value and not only a type, because a level whose title key is missing does
 * not fail loudly — it renders the key itself in the header, which is how
 * `appearance` first shipped. A test holds the two translations to this list.
 */
export const TITLED_SETTINGS_PAGES = [
  "preferences", "models", "skills", "tasks", "language", "appearance", "providers", "followAccount",
] as const;

type TitledSettingsPage = (typeof TITLED_SETTINGS_PAGES)[number];

/**
 * One screen level. Nested provider editors are levels too, so the system back
 * gesture and the header arrow always pop exactly one step — the same rule the
 * desktop dialog's single-level tabs cannot express.
 */
type SettingsRoute =
  | { name: "home" | TitledSettingsPage }
  | { name: "providerKey"; provider: RemoteBuiltinProvider }
  | { name: "providerForm"; provider: RemoteCustomProvider | null };

/**
 * Levels that write to the paired desktop.
 *
 * They carry a banner naming the machine, because the header at that point says
 * what is being configured and no longer which device: the levels below the
 * device home are the only place the two are separated.
 */
const DEVICE_LEVELS = new Set(["preferences", "models", "skills", "tasks", "providers"]);

/** A refused write, as the key of what failed. Held rather than a boolean
 * because the two surfaces fail differently and say so on screen. */
type SettingsNotice = "desktopSettings.saveFailed" | "desktops.actionFailed";

/**
 * The settings stack.
 *
 * Two entry points, one implementation. With no `deviceId` it is this phone's
 * own settings (reached from the session list header); with one it is that
 * paired desktop's page (reached from the paired list), which owns the
 * machine's identity *and* every setting that is read from or written to it.
 * A machine's settings are not offered anywhere else: the phone's own screen
 * must not silently edit whichever desktop happens to be connected.
 *
 * The device page reads the machine it names. Everything under it is scoped to
 * that desktop (`DesktopScope`, which its caller wraps it in), so the settings,
 * models and tasks on screen are that machine's, over that machine's own live
 * connection — which is why there is no "connect" step: every paired desktop is
 * already connected, and being the machine the phone is *reading* is a different
 * thing from being reachable.
 */
export function SettingsScreen({ deviceId, onClose, onCheckUpdate, checkingUpdate, onOpenConversation, ref }: {
  /** The paired desktop to configure. Omitted: this phone's own settings. */
  deviceId?: string;
  onClose(): void;
  /** Offered on the phone's own screen only. */
  onCheckUpdate?(): void;
  checkingUpdate?: boolean;
  /** Open a session in the chat (the caller closes settings first). */
  onOpenConversation?(sessionId: string): void;
  ref?: Ref<SettingsScreenHandle>;
}) {
  const settingsStyles = useSettingsStyles();
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const insets = useSafeAreaInsets();
  const remote = useRemoteControls();
  const [routes, setRoutes] = useState<SettingsRoute[]>([{ name: "home" }]);
  const page = routes[routes.length - 1]!;
  const push = useCallback((route: SettingsRoute) => setRoutes(current => [...current, route]), []);
  // The visible page may have a level of its own to leave first. Held in a ref
  // (read at call time) rather than state: registering is not a render input,
  // and a stale render's closure is exactly what this must not capture.
  const pageBackRef = useRef<SettingsPageBack | null>(null);
  const registerPageBack = useCallback((handle: SettingsPageBack | null) => {
    pageBackRef.current = handle;
  }, []);
  const goBack = useCallback(() => {
    if (pageBackRef.current?.goBack()) return;
    if (routes.length === 1) onClose();
    else setRoutes(current => current.slice(0, -1));
  }, [routes.length, onClose]);
  // A native Modal consumes Android's edge-back before BackHandler. Its owner
  // delegates here so system back and the header pop the same settings level.
  useImperativeHandle(ref, () => ({ goBack }), [goBack]);
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState<SettingsNotice | null>(null);
  const active = useRef(true);
  const writing = useRef(false);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const device = deviceId ? remote.desktops.find(item => item.desktopId === deviceId) : undefined;
  const deviceName = device ? device.name || device.desktopId : t("desktops.title");
  // The name is the phone's own label for the device: typed into a field, kept
  // here while the user edits it, and written when they leave the field.
  const [nameDraft, setNameDraft] = useState(device?.name ?? "");
  // The name this field last wrote. Dismissing the keyboard submits and then
  // blurs the field, and the second step must not write the same name again
  // while the list the field reads is still coming back.
  const writtenName = useRef<string | null>(null);
  // Which of the two surfaces this stack is: this phone's own settings, or one
  // paired desktop's.
  const deviceMode = deviceId !== undefined;
  const supported = remote.capabilities?.has("desktop_settings_v1") ?? false;
  const skillsSupported = remote.capabilities?.has("skill_management_v1") ?? false;
  const providersSupported = remote.capabilities?.has("provider_management_v1") ?? false;
  const tasksSupported = remote.capabilities?.has("tasks_v1") ?? false;
  const enabled = deviceMode && remote.desktopOnline && supported;
  const resource = useDesktopResource(remote.getDesktopSettings, remote.desktopSettingsRevision, enabled);
  const disabled = !enabled || saving || resource.loading || resource.failed || !resource.data;
  // Provider pages read and write their own authoritative snapshot; the shared
  // settings revision refetches them whenever the desktop changes a provider.
  const goBackFromEditor = useCallback(() => {
    setRoutes(current => current.slice(0, -1));
  }, []);

  const changeSettings = async (patch: Partial<DesktopSettings>) => {
    if (disabled || writing.current || !active.current) return;
    writing.current = true;
    setSaving(true);
    setFailed(null);
    try {
      await remote.updateDesktopSettings(patch);
    } catch {
      if (active.current) setFailed("desktopSettings.saveFailed");
    } finally {
      writing.current = false;
      if (active.current) {
        // Read back even on failure: a lost reply can follow a successful save.
        await resource.reload();
        if (active.current) setSaving(false);
      }
    }
  };
  const selectApproval = async (tier: string) => {
    if (!remote.desktopOnline || saving || writing.current) return;
    writing.current = true;
    setSaving(true);
    setFailed(null);
    try { await remote.setApprovalTier(tier); }
    catch { if (active.current) setFailed("desktopSettings.saveFailed"); }
    finally { writing.current = false; if (active.current) setSaving(false); }
  };

  /**
   * Write the typed name through the phone's own label store.
   *
   * A blank name clears the label, which is how a device goes back to being
   * listed by its id — the same rule the icon follows. A refused write puts the
   * stored name back in the field: the field must never show a name the store
   * does not have.
   */
  const saveName = async () => {
    if (!device) return;
    const next = nameDraft.trim();
    const stored = device.name ?? "";
    if (next === stored || next === writtenName.current) return;
    writtenName.current = next;
    try {
      await remote.renameDesktop(device.desktopId, next);
    } catch {
      if (active.current) {
        writtenName.current = null;
        setNameDraft(stored);
        setFailed("desktops.actionFailed");
      }
    }
  };

  /**
   * Pick this device's icon; picking the one it is already wearing clears it,
   * so the default is reachable without a separate "none" entry.
   */
  const chooseIcon = async (icon: string) => {
    if (!device) return;
    const next = device.icon === icon ? "" : icon;
    setFailed(null);
    try {
      await remote.labelDesktop(device.desktopId, { icon: next });
    } catch {
      if (active.current) setFailed("desktops.actionFailed");
    }
  };

  // Each level renders on its own; the stack keeps only the visible one mounted,
  // so returning to the provider list always re-reads the desktop's snapshot.
  function renderLevel() {
    switch (page.name) {
      case "providers":
        return <ProvidersSettingsPage
          onOpenBuiltin={provider => push({ name: "providerKey", provider })}
          onOpenCustom={provider => push({ name: "providerForm", provider })}
        />;
      case "providerKey":
        return <ProviderKeyPage key={page.provider.id} provider={page.provider} onSaved={goBackFromEditor} />;
      case "providerForm":
        return <CustomProviderPage key={page.provider?.id ?? "new"} onDone={goBackFromEditor} provider={page.provider} />;
      case "models":
        return enabled
          ? <ModelsSettingsPage settings={resource.data} disabled={disabled} onChange={patch => void changeSettings(patch)} />
          : null;
      case "skills":
        return remote.desktopOnline && skillsSupported ? <SkillsSettingsPage /> : null;
      case "tasks":
        return remote.desktopOnline && tasksSupported
          ? <TasksSettingsPage
              desktopOnline={remote.desktopOnline}
              settings={resource.data}
              onBackLevel={registerPageBack}
              onOpenConversation={onOpenConversation}
            />
          : null;
      case "language":
        return <ScrollView contentContainerStyle={settingsStyles.content}>
          <SettingsSection title={t("desktopSettings.thisPhone")}><View style={settingsStyles.card}><LanguageSettings /></View></SettingsSection>
        </ScrollView>;
      case "appearance":
        return <ScrollView contentContainerStyle={settingsStyles.content}>
          <SettingsSection title={t("desktopSettings.thisPhone")}><View style={settingsStyles.card}><AppearanceSettings /></View></SettingsSection>
        </ScrollView>;
      // Not scoped to the paired desktop: following the account needs no
      // connection, so this level stays usable while the desktop is offline.
      case "followAccount":
        return <FollowAccountPage />;
      case "preferences":
        return <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
          <SettingsSection title={t("desktopSettings.automation")}>
            {remote.desktopOnline && !supported ? <Text style={settingsStyles.description}>{t("desktopSettings.updateDesktop")}</Text> : null}
            <SettingsSwitch label={t("desktopSettings.autoUpgradeSkills")} description={t("desktopSettings.autoUpgradeSkillsHint")}
              value={resource.data?.autoUpgradeSkills ?? false} disabled={disabled} onChange={autoUpgradeSkills => void changeSettings({ autoUpgradeSkills })} />
            <SettingsSwitch label={t("desktopSettings.autoTitleFirstTurn")} description={t("desktopSettings.autoTitleFirstTurnHint")}
              value={resource.data?.autoTitleFirstTurn ?? false} disabled={disabled} onChange={autoTitleFirstTurn => void changeSettings({ autoTitleFirstTurn })} />
            {/* Default off when the desktop does not report it (an older desktop
                predates the toggle), matching the desktop's own default. */}
            <SettingsSwitch label={t("desktopSettings.skillRecommend")} description={t("desktopSettings.skillRecommendHint")}
              value={resource.data?.skillRecommend ?? false} disabled={disabled} onChange={skillRecommend => void changeSettings({ skillRecommend })} />
            {/* The remote desktop switch is deliberately absent: on that machine
                the Remote page's switch *is* the whole preference, and this row
                would write the value the host reads at launch, so a second
                control here would describe one state two ways (that switch keeps
                showing the old answer until a restart). It is changed on the
                desktop only. */}
            {enabled ? <ResourceStatus loading={resource.loading || saving} failed={resource.failed} onReload={() => void resource.reload()} /> : null}
          </SettingsSection>
          <SettingsSection title={t("approvalTier.title")}>
            <View accessibilityRole="radiogroup" style={settingsStyles.actions}>
              {(["manual", "sandbox", "auto", "off"] as const).filter(tier => (tier !== "sandbox" && tier !== "auto" || remote.sandboxAvailable) && (tier !== "auto" || remote.capabilities?.has("auto_approval_v1"))).map(tier =>
                <Pressable key={tier} accessibilityRole="radio" accessibilityLabel={t(`approvalTier.${tier}`)}
                  accessibilityState={{ checked: remote.approvalTier === tier, disabled: !remote.desktopOnline || saving }}
                  disabled={!remote.desktopOnline || saving} onPress={() => void selectApproval(tier)}
                  style={[styles.approval, remote.approvalTier === tier && styles.approvalSelected]}>
                  <Text style={settingsStyles.label}>{t(`approvalTier.${tier}`)}</Text>
                </Pressable>)}
            </View>
          </SettingsSection>
        </ScrollView>;
      default:
        return deviceMode ? renderDeviceHome() : renderPhoneHome();
    }
  }

  /**
   * This phone's own settings.
   *
   * Nothing here reaches a desktop: the machine's settings moved to its own
   * page, so this screen works with no desktop paired, offline, or asleep.
   */
  function renderPhoneHome() {
    return <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      <SettingsSection title={t("desktopSettings.thisPhone")}>
        <SettingsLink label={t("appearance.title")} onPress={() => push({ name: "appearance" })} />
        <SettingsLink label={t("language.title")} onPress={() => push({ name: "language" })} />
        {/* Deliberately never disabled: following the official account needs no
            desktop connection, unlike every other row that used to be here. */}
        <SettingsLink label={t("desktopSettings.followAccount")} onPress={() => push({ name: "followAccount" })} />
        {onCheckUpdate
          ? <SettingsLink label={t("update.check")} disabled={checkingUpdate} loading={checkingUpdate} onPress={onCheckUpdate} />
          : null}
      </SettingsSection>
      <Text style={styles.version}>{t("common.version", { version: VERSION })}</Text>
    </ScrollView>;
  }

  /**
   * One paired desktop: what it is called here, and what it does.
   *
   * The identity is local to this phone (name, icon) while everything below it
   * is read from the machine, so the two are one page but not one section.
   */
  function renderDeviceHome() {
    const capable = supported && skillsSupported && providersSupported && tasksSupported;
    return <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      <SettingsSection title={t("desktops.identity")}>
        <SettingsField hint={t("desktops.nameHint")} label={t("desktops.name")}>
          <TextInput
            accessibilityLabel={t("desktops.name")}
            autoCapitalize="none"
            autoCorrect={false}
            maxLength={MAX_NAME_LENGTH}
            onBlur={() => void saveName()}
            onChangeText={setNameDraft}
            onSubmitEditing={() => {
              Keyboard.dismiss();
              void saveName();
            }}
            placeholder={device?.desktopId}
            placeholderTextColor={colors.inkMuted}
            returnKeyType="done"
            selectTextOnFocus
            style={settingsStyles.input}
            value={nameDraft}
          />
        </SettingsField>
        {/* The icon replaces the name in the session list, so it is a sibling of
            it rather than part of it, and the two are cleared separately. */}
        <SettingsField hint={t("desktops.iconHint")} label={t("desktops.iconLabel")}>
          <View accessibilityRole="radiogroup" style={styles.iconGrid}>
            {PEER_ICONS.map(icon => {
              const selected = device?.icon === icon.id;
              return (
                <Pressable
                  accessibilityLabel={t(`desktops.icons.${icon.labelKey}`)}
                  accessibilityRole="radio"
                  accessibilityState={{ checked: selected }}
                  key={icon.id}
                  onPress={() => void chooseIcon(icon.id)}
                  style={({ pressed }) => [
                    styles.iconOption,
                    selected && styles.iconOptionSelected,
                    pressed && settingsStyles.pressed,
                  ]}
                >
                  <Text style={styles.iconGlyph}>{icon.glyph}</Text>
                </Pressable>
              );
            })}
          </View>
        </SettingsField>
      </SettingsSection>
      <SettingsSection title={t("desktops.deviceSettings")}>
        {/* The machine's own rows, on every paired machine. Each one keeps a
            runtime of its own (see `DesktopScope`), so a machine's settings are
            read and written over its own connection — there is no connection
            step to perform first, and nothing here edits another machine. A
            machine that is not answering says so below, in the notice, and the
            rows that need it are disabled. */}
        <SettingsLink label={t("desktopSettings.preferences")} onPress={() => push({ name: "preferences" })} />
        <SettingsLink label={t("desktopSettings.models")} disabled={!enabled} onPress={() => push({ name: "models" })} />
        <SettingsLink label={t("desktopSettings.providers")} disabled={!remote.desktopOnline || !providersSupported} onPress={() => push({ name: "providers" })} />
        <SettingsLink label={t("desktopSettings.skills")} disabled={!remote.desktopOnline || !skillsSupported} onPress={() => push({ name: "skills" })} />
        <SettingsLink label={t("desktopSettings.tasks")} disabled={!remote.desktopOnline || !tasksSupported} onPress={() => push({ name: "tasks" })} />
        {remote.desktopOnline && !capable ? <Text style={settingsStyles.description}>{t("desktopSettings.updateDesktop")}</Text> : null}
      </SettingsSection>
    </ScrollView>;
  }

  return <View
    onAccessibilityEscape={goBack}
    // JS insets rather than SafeAreaView: this stack is presented in a native
    // Modal, whose views are not descendants of the app's SafeAreaProvider, so
    // the native SafeAreaView contributes nothing on Android.
    style={[settingsStyles.page, { paddingTop: insets.top, paddingBottom: insets.bottom }]}
  >
    <View style={styles.column}>
    <View style={styles.header}>
      <Pressable accessibilityLabel={t("common.back")} accessibilityRole="button"
        onPress={goBack}
        style={({ pressed }) => [styles.headerButton, pressed && styles.pressed]}>
        <ArrowLeft size={22} color={colors.ink} />
      </Pressable>
      <Text accessibilityRole="header" numberOfLines={1} style={styles.title}>{page.name === "home"
        ? deviceMode ? deviceName : t("sessions.settings")
        : t(`desktopSettings.${page.name}`)}</Text>
    </View>
    {deviceMode && DEVICE_LEVELS.has(page.name) ? <View style={styles.deviceScope}>
      {/* The machine's own mark, as the device list and the session list draw
          it: these pages write to it, and it is the one place the user's icon
          for it should be recognisable. */}
      <View style={styles.deviceIcon}><Text style={styles.deviceGlyph}>{iconGlyph(device?.icon)}</Text></View>
      <View style={styles.heading}>
        <Text style={styles.deviceName}>{t("desktopSettings.boundDesktop", { name: deviceName })}</Text>
        <Text style={settingsStyles.description}>{t("desktopSettings.sharedHint")}</Text>
      </View>
    </View> : null}
    {failed ? <Text accessibilityRole="alert" style={[styles.notice, settingsStyles.error]}>{t(failed)}</Text> : null}
    {page.name === "models" && enabled && (resource.failed || resource.loading) ? <View style={styles.notice}>
      <ResourceStatus loading={resource.loading} failed={resource.failed} onReload={() => void resource.reload()} />
    </View> : null}
    {deviceMode && !remote.desktopOnline ? <Text style={styles.notice}>{t("desktopSettings.offline")}</Text> : null}
    {renderLevel()}
    </View>
  </View>;
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  column: { flex: 1, width: "100%", maxWidth: layout.formMaxWidth, alignSelf: "center", paddingTop: spacing.sm },
  header: { flexDirection: "row", alignItems: "center", gap: spacing.sm, minHeight: 56, paddingHorizontal: layout.gutter, marginBottom: spacing.md },
  heading: { flex: 1, minWidth: 0 },
  title: { flex: 1, color: theme.colors.inkStrong, fontSize: theme.font(22), fontWeight: "700" },
  headerButton: { width: layout.touchTarget, height: layout.touchTarget, borderRadius: radius.md, alignItems: "center", justifyContent: "center" },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  deviceScope: { flexDirection: "row", alignItems: "flex-start", gap: spacing.md, marginHorizontal: layout.gutter, padding: spacing.md, borderRadius: radius.lg, backgroundColor: theme.colors.accentSoft },
  deviceIcon: { width: 36, height: 36, borderRadius: radius.md, alignItems: "center", justifyContent: "center", backgroundColor: theme.colors.surface },
  deviceGlyph: { fontSize: theme.font(20) },
  deviceName: { color: theme.colors.inkStrong, fontSize: theme.font(14), fontWeight: "600", marginBottom: spacing.xs },
  version: { ...createSettingsStyles(theme).description, textAlign: "center" },
  notice: { color: theme.colors.inkMuted, padding: layout.gutter, fontSize: theme.font(13) },
  approval: { minHeight: layout.touchTarget, justifyContent: "center", padding: spacing.sm, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surface },
  approvalSelected: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  // One row of icons per device, wrapping for the narrowest phone: the same set
  // the desktop app offers, so a device looks the same in both clients.
  iconGrid: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
  iconOption: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
  },
  iconOptionSelected: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  iconGlyph: { fontSize: theme.font(20) },
}));
