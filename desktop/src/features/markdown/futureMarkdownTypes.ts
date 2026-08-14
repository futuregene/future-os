import type { ResolvedMarkdownReference } from "../../integrations/storage/markdownReferences";

export { referenceKey } from "@future-os/markdown";
export type {
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
