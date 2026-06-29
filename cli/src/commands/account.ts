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

async function platformPost<T>(
  url: string,
  apiKey: string,
  payload: unknown,
): Promise<T> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 30_000);

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${apiKey}`,
        "Content-Type": "application/json",
        Accept: "application/json",
      },
      body: JSON.stringify(payload),
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

async function accountProfile(): Promise<void> {
  const auth = await loadAccountAuth();
  const url = `${auth.platformUrl}/platform/v1/account/profile`;

  const profile = await platformGet<ProfileResponse>(url, auth.apiKey);

  console.log(`  Email:           ${profile.email}`);
  console.log(`  User ID:         ${profile.user_id}`);
  console.log(`  Email verified:  ${profile.email_verified}`);
  console.log(`  Created:         ${profile.created_at}`);
}

// ── Balance ───────────────────────────────────────────────────────────────────

interface BalanceResponse {
  balance_credits: number;
  currency?: string;
}

async function accountBalance(jsonFlag: boolean): Promise<void> {
  const auth = await loadAccountAuth();
  const url = `${auth.platformUrl}/platform/v1/account/balance`;

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

// ── Recharge ──────────────────────────────────────────────────────────────────

interface RechargeOrderResponse {
  id: string;
  order_no: string;
  channel: string;
  product: string;
  amount_cents: number;
  amount_credits: number;
  currency: string;
  subject: string;
  status: string;
  provider_trade_no?: string;
  provider_payload_json?: unknown;
  pay_url?: string;
  qr_code_url?: string;
  return_url?: string;
  expires_at: string;
  paid_at?: string;
  created_at: string;
  updated_at: string;
}

async function accountRecharge(
  amountYuan: number,
  channel: string,
): Promise<void> {
  if (amountYuan <= 0) {
    console.error("Error: --amount must be a positive number (in CNY).");
    process.exitCode = 1;
    return;
  }

  const channelLower = channel.toLowerCase();
  if (channelLower !== "alipay" && channelLower !== "wechat") {
    console.error(
      'Error: --channel must be "alipay" or "wechat".',
    );
    process.exitCode = 1;
    return;
  }

  const auth = await loadAccountAuth();
  const url = `${auth.platformUrl}/platform/v1/account/recharge/orders`;

  // Convert yuan to cents: 1 CNY = 100 cents
  const amountCents = Math.round(amountYuan * 100);

  const order = await platformPost<RechargeOrderResponse>(url, auth.apiKey, {
    amount_cents: amountCents,
    channel: channelLower,
  });

  console.log(`  Order:     ${order.order_no}`);
  console.log(`  Amount:    ${(order.amount_cents / 100).toFixed(2)} CNY`);
  console.log(`  Channel:   ${order.channel}`);
  console.log(`  Status:    ${order.status}`);
  if (order.pay_url) {
    console.log(`  Pay URL:   ${order.pay_url}`);
  } else {
    console.log(`  Pay URL:   (not available yet)`);
  }
  console.log(`  Expires:   ${order.expires_at}`);
  console.log(`  Created:   ${order.created_at}`);
}

// ── Public command ───────────────────────────────────────────────────────────

export type AccountCommand = "profile" | "balance" | "recharge";

export function isAccountCommand(command: string): command is AccountCommand {
  return command === "profile" || command === "balance" || command === "recharge";
}

export async function account(
  command: AccountCommand,
  args: string[],
): Promise<void> {
  switch (command) {
    case "profile": {
      await accountProfile();
      return;
    }
    case "balance": {
      const jsonFlag = args.includes("--json");
      await accountBalance(jsonFlag);
      return;
    }
    case "recharge": {
      const amountIdx = args.indexOf("--amount");
      const channelIdx = args.indexOf("--channel");

      if (amountIdx === -1 || amountIdx + 1 >= args.length) {
        console.error(
          'Usage: future account recharge --amount <yuan> --channel <alipay|wechat>',
        );
        process.exitCode = 1;
        return;
      }

      const amountYuan = Number(args[amountIdx + 1]);
      if (Number.isNaN(amountYuan)) {
        console.error(
          `Error: --amount must be a number, got "${args[amountIdx + 1]}".`,
        );
        process.exitCode = 1;
        return;
      }

      if (channelIdx === -1 || channelIdx + 1 >= args.length) {
        console.error(
          'Usage: future account recharge --amount <yuan> --channel <alipay|wechat>',
        );
        process.exitCode = 1;
        return;
      }

      const channel = args[channelIdx + 1];
      await accountRecharge(amountYuan, channel);
      return;
    }
  }
}
