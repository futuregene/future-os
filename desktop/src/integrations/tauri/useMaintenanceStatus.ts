import { useEffect, useState } from "react";
import { errorMessage } from "../../lib/errors";
import { invokeCommand } from "./invoke";

export interface MaintenanceStatus {
  cli: { error?: string; tool?: { path: string; source: string; version: string } };
  update: {
    operationId: string;
    fromVersion: string;
    targetVersion: string | null;
    stage: string;
    outcome: string | null;
    diagnostic: string | null;
    restartSafe: boolean;
    repairAcknowledged: boolean;
  } | null;
}

/** Native maintenance queries are independent of Agent business admission. */
export function useMaintenanceStatus() {
  const [maintenance, setMaintenance] = useState<MaintenanceStatus | null>(null);
  const [maintenanceError, setMaintenanceError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let querying = false;
    const refresh = async () => {
      if (querying)
        return;
      querying = true;
      try {
        const result = await invokeCommand<MaintenanceStatus>("app_maintenance_status");
        if (!disposed) {
          setMaintenance(result ?? null);
          setMaintenanceError(null);
        }
      }
      catch (error) {
        if (!disposed)
          setMaintenanceError(errorMessage(error));
      }
      finally {
        querying = false;
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, []);

  return { maintenance, setMaintenance, maintenanceError, setMaintenanceError };
}
