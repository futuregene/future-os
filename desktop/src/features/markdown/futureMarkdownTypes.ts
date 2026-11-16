import type { ResolvedMarkdownReference } from "../../integrations/storage/markdownReferences";

export { referenceKey } from "@future-os/markdown";
export type {
  ChoiceCardMode,
  ChoiceCardNode,
  ChoiceCardOption,
  FutureMarkdownDocument,
  FutureReference,
  FutureReferenceType,
  FutureReferenceView,
  InlineNode,
  ListItemNode,
  MarkdownNode,
  TableNode,
} from "@future-os/markdown";

export type ResolvedReferenceMap = Record<string, ResolvedMarkdownReference>;
