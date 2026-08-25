export interface ApprovalDecisionDisabled {
  approved: boolean;
  rejected: boolean;
}

/**
 * A malformed capability must remain rejectable, but no approval path may be
 * available when the trusted target list cannot be rendered.
 */
export function approvalDecisionDisabled(
  submitting: boolean,
  malformedCapability: boolean,
): ApprovalDecisionDisabled {
  return {
    approved: submitting || malformedCapability,
    rejected: submitting,
  };
}
