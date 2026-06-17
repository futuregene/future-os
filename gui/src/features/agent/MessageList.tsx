import type { AgentMessage } from "./types";
import { MessageBlock } from "./MessageBlock";

interface MessageListProps {
  messages: AgentMessage[];
  workspaceId?: string | null;
}

export function MessageList({ messages, workspaceId }: MessageListProps) {
  return (
    <div className="space-y-5">
      {messages.map(message => (
        <MessageBlock key={message.id} message={message} workspaceId={workspaceId} />
      ))}
    </div>
  );
}
