import { readFile } from "node:fs/promises";

import { AUTH_FILE, FUTURE_AUTH_PROVIDER } from "../constants.js";
import { isRecord, isNodeError } from "../utils/object.js";
import { getPlatformUrl } from "../utils/platform.js";

// ── Auth helpers ──────────────────────────────────────────────────────────────

interface AccountAuth {
  apiKey: string;
  platformUrl: string;
}

async function loadAccountAuth(): Promise<AccountAuth> {
  let raw: string;
  try {
    raw = await readFile(AUTH_FILE, "utf8");
  } catch (error) {
    if (isNodeError(error) && error.code === "ENOENT") {
      throw new Error(
        `No API key found. Run "future auth login" first, or set FUTURE_API_KEY.`,
      );
    }
    throw error;
  }

  const parsed = JSON.parse(raw) as unknown;
  if (!isRecord(parsed)) {
    throw new Error(`${AUTH_FILE} must contain a JSON object.`);
  }

  const future = parsed[FUTURE_AUTH_PROVIDER];
  if (!isRecord(future)) {
    throw new Error(`No "${FUTURE_AUTH_PROVIDER}" provider in ${AUTH_FILE}.`);
  }

  const key =
    typeof (future as Record<string, unknown>).key === "string"
      ? ((future as Record<string, unknown>).key as string)
      : undefined;
  if (!key) {
    throw new Error(
      `No API key for "${FUTURE_AUTH_PROVIDER}" in ${AUTH_FILE}. Run "future auth login" first.`,
    );
  }

  return {
    apiKey: key,
    platformUrl: await getPlatformUrl(),
  };
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

async function platformGet<T>(url: string, apiKey: string): Promise<T> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 30_000);

  try {
    const response = await fetch(url, {
      method: "GET",
      headers: {
        Authorization: `Bearer ${apiKey}`,
        Accept: "application/json",
      },
      signal: controller.signal,
    });

    const body = (await response.json()) as { error?: string; message?: string };
    if (!response.ok) {
      throw new Error(
        body.message ?? body.error ?? `HTTP ${response.status}`,
      );
    }
    return body as T;
  } finally {
    clearTimeout(timeout);
  }
}

// ── Profile ───────────────────────────────────────────────────────────────────

interface ProfileResponse {
  user_id: string;
  email: string;
  email_verified: boolean;
  created_at: string;
}

async function accountProfile(jsonFlag: boolean): Promise<void> {
  const auth = await loadAccountAuth();
  const url = `${auth.platformUrl}/client/v1/account/profile`;

  const profile = await platformGet<ProfileResponse>(url, auth.apiKey);

  if (jsonFlag) {
    console.log(
      JSON.stringify(
        {
          email: profile.email,
          user_id: profile.user_id,
          email_verified: profile.email_verified,
          created_at: profile.created_at,
        },
        null,
        2,
      ),
    );
  } else {
    console.log(`  Email:           ${profile.email}`);
    console.log(`  User ID:         ${profile.user_id}`);
    console.log(`  Email verified:  ${profile.email_verified}`);
    console.log(`  Created:         ${profile.created_at}`);
  }
}

// ── Balance ───────────────────────────────────────────────────────────────────

interface BalanceResponse {
  balance_credits: number;
  currency?: string;
}

async function accountBalance(jsonFlag: boolean): Promise<void> {
  const auth = await loadAccountAuth();
  const url = `${auth.platformUrl}/client/v1/account/balance`;

  const balance = await platformGet<BalanceResponse>(url, auth.apiKey);

  // balance_credits is in internal units: 1 credit = 10,000,000,000 units
  const credits = balance.balance_credits / 10_000_000_000;

  if (jsonFlag) {
    console.log(
      JSON.stringify(
        {
          balance_credits: balance.balance_credits,
          credits: Number(credits.toFixed(3)),
        },
        null,
        2,
      ),
    );
  } else {
    console.log(`  Balance: ${credits.toFixed(3)} credits`);
  }
}

// ── Public command ───────────────────────────────────────────────────────────

export type AccountCommand = "profile" | "balance";

export function isAccountCommand(command: string): command is AccountCommand {
  return command === "profile" || command === "balance";
}

export async function account(
  command: AccountCommand,
  args: string[],
): Promise<void> {
  switch (command) {
    case "profile": {
      const jsonFlag = args.includes("--json");
      await accountProfile(jsonFlag);
      return;
    }
    case "balance": {
      const jsonFlag = args.includes("--json");
      await accountBalance(jsonFlag);
      return;
    }
  }
}
