import type { AgentMessage } from "./agentThreadTypes";
import { MessageBlock } from "./MessageBlock";

interface MessageListProps {
  messages: AgentMessage[];
  onContinue?: (message: AgentMessage) => void;
  onRetry?: (message: AgentMessage, source: AgentMessage) => void;
  workspaceId?: string | null;
}

export function MessageList({ messages, onContinue, onRetry, workspaceId }: MessageListProps) {
  return (
    <div className="space-y-5">
      {messages.map((message, index) => (
        <MessageBlock
          key={message.id}
          message={message}
          isLast={index === messages.length - 1}
          recoverySource={previousUserMessage(messages, index)}
          workspaceId={workspaceId}
          onContinue={onContinue}
          onRetry={onRetry}
        />
      ))}
    </div>
  );
}

function previousUserMessage(messages: AgentMessage[], index: number) {
  for (let cursor = index - 1; cursor >= 0; cursor -= 1) {
    const message = messages[cursor];
    if (message?.role === "user") {
      return message;
    }
  }
  return null;
}
