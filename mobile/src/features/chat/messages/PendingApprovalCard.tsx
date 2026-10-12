import { AlertTriangle, Check, ChevronDown, ChevronUp, X } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { approvalCommand, approvalDeletes, approvalPaths, escalationTitleKey, parseAction } from "@future-os/thread-projection";
import type { ApprovalPayload } from "../../../remote/types";
import { colors } from "../../../theme/tokens";
import { Button } from "../../../components/Button";
import { approvalDecisionDisabled } from "../../../components/approvalState";
import { styles } from "./messageStyles";

// Localized title/summary per approval kind — the agent ships English, so map
// by kind exactly like the desktop ApprovalPrompt, falling back to the wire
// strings for any unknown kind.
const APPROVAL_KIND_I18N: Record<string, { title: string; summary?: string; }> = {
  file_read: { title: "approval.readTitle", summary: "approval.readSummary" },
  file_write: { title: "approval.writeTitle", summary: "approval.writeSummary" },
  outside_workspace_write: {
    title: "approval.outsideWriteTitle",
    summary: "approval.outsideWriteSummary",
  },
  shell_command: { title: "approval.shellTitle", summary: "approval.shellSummary" },
  sandbox_escalation: { title: "approval.escalationTitle" },
};

interface PendingApprovalCardProps {
  payload: ApprovalPayload;
  submitting: boolean;
  error: string | null;
  onDecision(decision: "approved" | "rejected"): void;
}

// Desktop-styled approval card (neutral surface + warning dot, not the old
// cream/warning fill). Rendered docked above the composer; it vanishes as soon
// as a decision lands because the caller only feeds undecided approvals.
export function PendingApprovalCard({
  payload,
  submitting,
  error,
  onDecision,
}: PendingApprovalCardProps) {
  const { t } = useTranslation();
  const [commandExpanded, setCommandExpanded] = useState(false);
  const action = parseAction(payload.action);
  const capabilityTargets =
    action?.category === "windows_write_capability" ? action.targets : undefined;
  const capabilityTarget = capabilityTargets?.length === 1 ? capabilityTargets[0] : undefined;
  const malformedCapability = payload.kind === "windows_write_capability" && !capabilityTargets;
  const decisionDisabled = approvalDecisionDisabled(submitting, malformedCapability);
  const kindI18n = APPROVAL_KIND_I18N[payload.kind ?? ""];
  const titleText = capabilityTargets
    ? capabilityTarget
      ? t(
        capabilityTarget.scope === "file"
          ? "approval.capabilityFileTitle"
          : "approval.capabilitySubtreeTitle",
        { path: capabilityTarget.path },
      )
      : t("approval.capabilityMultiTitle", { count: capabilityTargets.length })
    : kindI18n
      ? t(payload.kind === "sandbox_escalation" ? escalationTitleKey(action) : kindI18n.title)
      : payload.title || payload.tool_name || t("approval.title");
  const summaryText = capabilityTargets
    ? null
    : kindI18n?.summary
      ? t(kindI18n.summary)
      : payload.summary;
  const paths = approvalPaths(payload);
  const deletes = approvalDeletes(payload);
  const command = approvalCommand(payload);
  const isWrite =
    paths.length > 0 &&
    (payload.kind === "file_write" || payload.kind === "outside_workspace_write");
  const detailLabel = capabilityTargets
    ? capabilityTargets.length > 1
      ? t("approval.locations")
      : null
    : deletes.length > 0
      ? deletes.length === 1
        ? t("approval.deleteFile")
        : t("approval.deleteFiles", { count: deletes.length })
      : paths.length > 0
        ? isWrite
          ? paths.length === 1
            ? t("approval.writeFile")
            : t("approval.writeFiles", { count: paths.length })
          : t("approval.readPath")
        : command
          ? t("approval.command")
          : null;

  return (
    <View style={styles.approval}>
      <View style={styles.approvalHeader}>
        <View style={styles.approvalDot} />
        <Text numberOfLines={2} style={styles.approvalTitle}>
          {titleText}
        </Text>
      </View>
      {!!summaryText && <Text style={styles.approvalSummary}>{summaryText}</Text>}
      {detailLabel ? (
        <>
          <Text style={styles.approvalDetailLabel}>{detailLabel}</Text>
          <View style={styles.approvalDetail}>
            {capabilityTargets ? (
              capabilityTargets.map((target, index) => (
                <Text key={`${target.path}-${index}`} selectable style={styles.approvalPath}>
                  {target.path}
                </Text>
              ))
            ) : command && deletes.length === 0 ? (
              <Text selectable style={styles.approvalPath}>
                {command}
              </Text>
            ) : (
              (deletes.length > 0 ? deletes : paths).map((path, index) => (
                <Text
                  key={`${path}-${index}`}
                  numberOfLines={2}
                  selectable
                  style={styles.approvalPath}
                >
                  {path}
                </Text>
              ))
            )}
          </View>
        </>
      ) : null}
      {capabilityTargets && command ? (
        <View style={styles.approvalDetail}>
          <Pressable
            accessibilityRole="button"
            onPress={() => setCommandExpanded(value => !value)}
            style={styles.approvalCommandToggle}
          >
            <Text style={styles.approvalDetailLabel}>{t("approval.viewCommand")}</Text>
            {commandExpanded ? (
              <ChevronUp color={colors.inkMuted} size={14} />
            ) : (
              <ChevronDown color={colors.inkMuted} size={14} />
            )}
          </Pressable>
          {commandExpanded ? (
            <Text selectable style={styles.approvalPath}>
              {command}
            </Text>
          ) : null}
        </View>
      ) : null}
      {malformedCapability ? (
        <View style={styles.approvalError}>
          <AlertTriangle color={colors.danger} size={13} />
          <Text style={styles.approvalErrorText}>{t("approval.invalidRequest")}</Text>
        </View>
      ) : null}
      {!!error && (
        <View style={styles.approvalError}>
          <AlertTriangle color={colors.danger} size={13} />
          <Text style={styles.approvalErrorText}>{error}</Text>
        </View>
      )}
      <View style={styles.approvalActions}>
        <View style={styles.approvalActionLeft}>
          <Button
            compact
            disabled={decisionDisabled.rejected}
            icon={<X color={colors.ink} size={15} />}
            label={submitting ? t("approval.denying") : t("approval.deny")}
            loading={submitting}
            onPress={() => onDecision("rejected")}
            variant="secondary"
          />
        </View>
        <View style={styles.approvalActionRight}>
          <Button
            compact
            disabled={decisionDisabled.approved}
            icon={<Check color={colors.surface} size={15} />}
            label={submitting ? t("approval.allowing") : t("approval.allowOnce")}
            loading={submitting}
            onPress={() => onDecision("approved")}
            variant="primary"
          />
        </View>
      </View>
    </View>
  );
}
