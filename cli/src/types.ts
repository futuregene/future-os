export type ServiceResult = {
  code: number;
  stdout: string;
  stderr: string;
};

export type AgentCommand = "start" | "stop" | "restart" | "status";
