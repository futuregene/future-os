/**
 * `future agent` command — show running agent status.
 */

import { RunClient } from "../rpc/grpc-client.js";

const grpcAddr = () => process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051";

export async function agentStatus(jsonFlag: boolean): Promise<void> {
  const client = new RunClient(grpcAddr());

  let info: Awaited<ReturnType<typeof client.getAgentInfo>>;
  try {
    info = await client.getAgentInfo();
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (jsonFlag) console.log(JSON.stringify({ error: msg }));
    else console.error(`Error: ${msg}`);
    process.exit(1);
  }

  if (jsonFlag) {
    console.log(JSON.stringify(info, null, 2));
    return;
  }

  console.log(`  Version:  ${info.version}`);
  console.log(`  Skills:   ${info.skillsCount} loaded`);
}
