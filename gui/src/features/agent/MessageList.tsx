import type { AgentMessage } from "./types";
import { MessageBlock } from "./MessageBlock";

interface MessageListProps {
  messages: AgentMessage[];
}

export function MessageList({ messages }: MessageListProps) {
  return (
    <div className="space-y-5">
      {messages.map(message => (
        <MessageBlock key={message.id} message={message} />
      ))}
    </div>
  );
}
