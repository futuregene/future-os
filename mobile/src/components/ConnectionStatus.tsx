import { useTranslation } from "react-i18next";
import { StyleSheet, Text, View } from "react-native";
import type { ConnectionPresentation } from "../remote/connectionPresentation";
import type { MachineStatus, MachineSummary } from "../remote/desktopStatus";
import { machinesConnected, machinesPartlyConnected } from "../remote/desktopStatus";
import { disconnectedCopy } from "../remote/disconnectedCopy";
import { radius, spacing, type Palette } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { Button } from "./Button";

/** The dot a machine's own row carries, in the same palette as the header's. */
export function machineStatusColor(status: MachineStatus, colors: Palette): string {
  if (status === "connected") return colors.success;
  return status === "offline" ? colors.danger : colors.warningDot;
}

/**
 * A machine's dot: answered, and ours to drive, or held by another device.
 *
 * A machine another device holds answers every heartbeat and still cannot be
 * read from, so its row wears the same yellow the header's dot does rather than
 * the green of "connected".
 */
export function machineDotColor(
  machine: Pick<MachineSummary, "status" | "control">,
  colors: Palette,
): string {
  const held = machine.status === "connected" && machine.control === "standby";
  return held ? colors.warningDot : machineStatusColor(machine.status, colors);
}

/**
 * The three states the dot and the status title share, and the two hues they
 * wear.
 *
 * The dot is yellow at `partial`: it and the title are drawn on the same light
 * surface, but one is an 8px shape and the other a line of text, and the hues
 * those two want are not the same. `warning` is darkened so it can be read as
 * text; at the size of a dot that same orange is close enough to the `danger`
 * red of the state below that "some machines are down" reads as "all of them
 * are" — the one distinction this dot exists to make. So the dot takes the
 * yellow, and the text keeps the amber it has contrast for.
 */
export type ConnectionState = "up" | "partial" | "down";

export function connectionStateColors(colors: Palette): Record<ConnectionState, { dot: string; text: string }> {
  return {
    up: { dot: colors.success, text: colors.success },
    partial: { dot: colors.warningDot, text: colors.warning },
    down: { dot: colors.danger, text: colors.danger },
  };
}

/**
 * How the phone stands with its machines, as one word for the dot's colour.
 *
 * "Connected" alone is not green: a machine that answers while another paired
 * device holds it cannot be read from or commanded, and painting that green
 * told the user everything was fine while every action failed. Green means
 * every machine is connected *and* ours to drive; one that is held makes the
 * whole set yellow, exactly as one that is offline does. "Somewhere to work"
 * is the rows' answer, not the header's: the dot answers for the set, and a
 * machine in standby is one this phone cannot use.
 *
 * With no machine list (nothing paired but the open connection) the single
 * connection the app renders from answers instead.
 */
export function connectionState({
  machines,
  presentation,
  standby = false,
}: {
  machines?: readonly MachineSummary[];
  presentation: ConnectionPresentation;
  standby?: boolean;
}): ConnectionState {
  const summary = machines && machines.length > 0 ? machinesConnected(machines) : null;
  if (summary) {
    return summary.all
      ? (summary.drivable === summary.total ? "up" : "partial")
      : summary.any
        ? "partial"
        : "down";
  }
  if (presentation.level === "connected") return standby ? "partial" : "up";
  return presentation.level === "connecting" ? "partial" : "down";
}

/**
 * Everything the status block and the header's dot read, in one place.
 *
 * Both surfaces sit in the same sheet — the dot is the trigger and the block is
 * what it opens — so they must never disagree about what the phone's state is
 * called or which hue it wears.
 */
export interface ConnectionFacts {
  presentation: ConnectionPresentation;
  disconnectReason?: string;
  error?: string | null;
  /**
   * Whether the machine the badge is rendered for is held by another device.
   *
   * Only read when there is no machine list to judge (the single-device
   * fallback): with a list, every machine's own `control` answers this.
   */
  standby?: boolean;
  /**
   * The device holding the machine, when another one does. Only used to name it
   * in the block's own words; the per-machine rows read their own state.
   */
  holder?: { name?: string } | null;
  onReconnect?: () => void;
  onUnpair?: () => void;
}

export interface ConnectionSummary {
  state: ConnectionState;
  /** The status as one phrase: the trigger's own name, and the block's title. */
  label: string;
  /** Why it stands that way, support code included where there is one. */
  reason: string;
  /** The next step, when there is one to state. */
  solution: string;
  /** More than one machine is paired, so the action reaches all of them. */
  severalMachines: boolean;
  /** Whether a manual reconnect is the offer. */
  reconnectable: boolean;
  /** Whether the pairing is gone and pairing again is the only way back. */
  unpairable: boolean;
}

/**
 * The phone's connection, in the words both surfaces show.
 *
 * `machines` answers for the whole set — a phone paired to three desktops is
 * green only when all three are connected, because a merged list that is
 * missing a machine is exactly the state the reader has to be able to see — and
 * the open machine's own connection fills in the explanation when the set
 * cannot: nothing answering at all, or a pairing this phone has lost.
 */
export function useConnectionSummary({
  machines,
  presentation,
  disconnectReason,
  error,
  standby = false,
  holder = null,
}: ConnectionFacts & { machines?: readonly MachineSummary[] }): ConnectionSummary {
  const { t } = useTranslation();
  const severalMachines = (machines?.length ?? 0) > 1;
  const summary = machines ? machinesConnected(machines) : null;
  // ...and the block under the rows answers for the same set. A phone paired to
  // three desktops, two of them up, is not "no connection" — which is what the
  // one machine's own state says when that machine is the one that is down.
  const partial = summary !== null && machinesPartlyConnected(summary, presentation);
  const state = connectionState({ machines, presentation, standby });
  // A connection the app still reports as healthy (or still trying) is where a
  // silently dead transport hides: those states keep the same manual reconnect
  // the failure states get, so recovering never requires unpairing first.
  const reconnectable = (
    presentation.action === "reconnect" ||
    presentation.action === "retry" ||
    presentation.action === "checkNetwork" ||
    presentation.action === "none" ||
    presentation.action === "wait"
  );
  const unpairable = presentation.action === "pairAgain";
  const disconnected = presentation.level === "disconnected";
  const copy = disconnectedCopy(presentation, disconnectReason, error);
  const statusKey = disconnected
    ? presentation.customerState === "pairingExpired"
      ? "connection.pairingExpired"
      : "connection.disconnected"
    : presentation.titleKey;
  const detailState = presentation.customerState === "devicePreparing"
    ? "devicePreparing"
    : presentation.customerState === "waitingDesktop"
      ? "waitingDesktop"
      : presentation.level;
  // The machine the screen is about, by its own flag rather than by position:
  // the list is built active-first today, but this reading must not depend on it.
  const activeMachine = machines?.find(entry => entry.active) ?? machines?.[0] ?? null;
  const activeHeld = activeMachine
    ? activeMachine.status === "connected" && activeMachine.control === "standby"
    : standby && presentation.level === "connected";
  // The machine the phone is reading answers every heartbeat and still cannot be
  // used: another paired device holds it. The dot is yellow for the whole set,
  // but the block's own words are the open machine's — that is where the reason
  // belongs, with the holder's name when it gave one. `partial` (some machines
  // are down) outranks it: that is the more urgent fact.
  const standbyHere = activeHeld && !partial;
  // The holder belongs to the open machine (`remote.presence`), so it names the
  // device this phone would displace even when peers are paired too.
  const holderName = standbyHere ? holder?.name?.trim() ?? null : null;
  const label = partial
    ? t("connection.machinesPartial")
    : standbyHere
      ? t("connection.connectedStandby")
      : t(statusKey);
  const reason = partial
    ? t("connection.machinesSummary", {
      connected: summary!.connected,
      total: summary!.total,
    })
    : standbyHere
      ? (holderName
          ? t("connection.controlHeldBy", { name: holderName })
          : t("connection.controlHeldByUnknown"))
      : disconnected
        ? t(`connection.disconnectDetails.${copy}.reason`)
        : t(`connection.statusDetails.${detailState}.reason`);
  const solution = partial
    ? t("connection.machinesPartialHint")
    : standbyHere
      ? (holderName
          ? t("connection.takeControlHint", { name: holderName })
          : t("connection.takeControlHintUnknown"))
      : disconnected
        ? t(`connection.disconnectDetails.${copy}.solution`)
        : t(`connection.statusDetails.${detailState}.solution`);
  // The set's own state carries no support code: there is no single failure to
  // look up, and the rows below name which machines it is about. The open
  // machine's code is still the phone's when nothing answers at all, where
  // `partial` is false.
  const reasonWithCode = partial
    ? reason
    : reason + (presentation.supportCode ? ` (${presentation.supportCode})` : "");

  return {
    state,
    label,
    reason: reasonWithCode,
    solution,
    severalMachines,
    reconnectable,
    unpairable,
  };
}

/**
 * The connection, explained, inside the sheet that names the machines.
 *
 * This used to be a popover of its own behind a bare status dot. The dot, the
 * machine list and the actions are one question — "which of my desktops can I
 * use, and what do I do about the ones I cannot" — so they are one sheet: the
 * dot now sits on the device trigger, and this block is the answer it opens
 * with. It carries no per-machine rows any more: those are the trigger's own
 * rows below it, each with its own dot and state, rather than a second list of
 * the same machines a few lines apart.
 */
export function ConnectionStatusBlock({
  machines,
  onReconnect,
  onUnpair,
  ...facts
}: ConnectionFacts & { machines?: readonly MachineSummary[] }) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const { state, label, reason, solution, severalMachines, reconnectable, unpairable }
    = useConnectionSummary({ machines, onReconnect, onUnpair, ...facts });
  const { text: titleColor } = connectionStateColors(colors)[state];
  const showAction = (reconnectable && !!onReconnect) || (unpairable && !!onUnpair);

  return (
    <View style={styles.block}>
      <Text accessibilityLiveRegion="polite" style={[styles.title, { color: titleColor }]}>{label}</Text>
      <Text style={styles.hint}>{reason}</Text>
      {solution ? <Text style={styles.hint}>{solution}</Text> : null}
      {showAction && (
        <Button
          compact
          variant={facts.presentation.level === "disconnected" ? "primary" : "secondary"}
          label={t(unpairable
            ? "sessions.unpair"
            : severalMachines ? "connection.reconnectAll" : "connection.reconnect")}
          onPress={() => {
            if (unpairable) onUnpair?.();
            else onReconnect?.();
          }}
        />
      )}
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  block: {
    gap: spacing.xs,
    padding: spacing.md,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surfaceSubtle,
  },
  title: { fontSize: theme.font(15), fontWeight: "600" },
  hint: { color: theme.colors.inkSoft, fontSize: theme.font(14), lineHeight: theme.leading(21) },
}));
