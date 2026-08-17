import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { connect } from "node:net";

export type Money = { minor: number; currency: string };
export type PurchaseIntentResult = Record<string, unknown>;
export type TransactionPage = {
  transactions: PurchaseIntentResult[];
  transactions_total: number;
  next_cursor: string | null;
  has_more: boolean;
};

export class BrokerError extends Error {
  constructor(message: string, readonly response?: unknown) {
    super(message);
    this.name = "BrokerError";
  }
}

type Operation = Record<string, unknown>;
type RpcResponse = {
  api_version: string;
  request_id: string;
  ok: boolean;
  data?: unknown;
  error?: string;
};

export type BrokerClientOptions = {
  socketPath: string;
  tokenFile: string;
  timeoutMs?: number;
};

function assertBounded(value: string, field: string, max: number): void {
  if (value.length < 1 || value.length > max || /[\u0000-\u001f\u007f]/u.test(value)) {
    throw new TypeError(`${field} must contain 1..${max} printable characters`);
  }
}

function assertMoney(value: Money, field: string): void {
  if (!Number.isSafeInteger(value.minor) || value.minor <= 0) {
    throw new TypeError(`${field}.minor must be a positive safe integer`);
  }
  if (!/^[A-Z]{3}$/u.test(value.currency)) {
    throw new TypeError(`${field}.currency must be an uppercase ISO 4217 code`);
  }
}

export class BrokerClient {
  private readonly token: string;
  private readonly timeoutMs: number;

  constructor(private readonly options: BrokerClientOptions) {
    assertBounded(options.socketPath, "socketPath", 4096);
    assertBounded(options.tokenFile, "tokenFile", 4096);
    this.token = readFileSync(options.tokenFile, "utf8").trim();
    assertBounded(this.token, "capability token", 128);
    this.timeoutMs = options.timeoutMs ?? 10_000;
  }

  async request<T = unknown>(operation: Operation): Promise<T> {
    const request = JSON.stringify({
      api_version: "v1",
      request_id: randomUUID(),
      token: this.token,
      operation,
    });
    return await new Promise<T>((resolve, reject) => {
      const socket = connect({ path: this.options.socketPath });
      let buffer = "";
      let settled = false;
      const timer = setTimeout(() => {
        finish(new BrokerError("broker request timed out"));
        socket.destroy();
      }, this.timeoutMs);

      const finish = (error?: Error, value?: T): void => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        socket.destroy();
        if (error) reject(error);
        else resolve(value as T);
      };

      socket.on("error", (error) => finish(new BrokerError(`broker connection failed: ${error.message}`)));
      socket.on("data", (chunk: Buffer) => {
        buffer += chunk.toString("utf8");
        if (buffer.length > 256 * 1024) {
          finish(new BrokerError("broker response is too large"));
          return;
        }
        const newline = buffer.indexOf("\n");
        if (newline < 0) return;
        const line = buffer.slice(0, newline);
        let response: RpcResponse;
        try {
          response = JSON.parse(line) as RpcResponse;
        } catch (error) {
          finish(new BrokerError(`broker returned invalid JSON: ${String(error)}`));
          return;
        }
        if (!response.ok) {
          finish(new BrokerError(response.error ?? "broker rejected request", response));
          return;
        }
        finish(undefined, response.data as T);
      });
      socket.on("connect", () => socket.write(`${request}\n`));
    });
  }

  getStatus(): Promise<Record<string, unknown>> {
    return this.request({ type: "get_status" });
  }

  getCapabilities(): Promise<Record<string, unknown>> {
    return this.request({ type: "get_capabilities" });
  }

  getBudget(): Promise<Record<string, unknown>> {
    return this.request({ type: "get_budget" });
  }

  getReceiveInstructions(): Promise<Record<string, unknown>> {
    return this.request({ type: "get_receive_instructions" });
  }

  createPurchaseIntent(request: Record<string, unknown>): Promise<PurchaseIntentResult> {
    const amount = request.amount as Money;
    const finalTotal = request.final_total as Money;
    assertMoney(amount, "amount");
    assertMoney(finalTotal, "final_total");
    if (amount.currency !== finalTotal.currency) {
      throw new TypeError("amount and final_total currencies must match");
    }
    for (const [field, max] of [["idempotency_key", 128], ["merchant_domain", 253], ["category", 64], ["fulfillment_profile", 64], ["session_id", 128]] as const) {
      assertBounded(String(request[field]), field, max);
    }
    const items = request.items;
    if (!Array.isArray(items) || items.length < 1 || items.length > 50) {
      throw new TypeError("items must contain 1..50 entries");
    }
    for (const item of items as Array<Record<string, unknown>>) {
      assertBounded(String(item.label), "item.label", 160);
      if (!Number.isInteger(item.quantity) || Number(item.quantity) <= 0 || Number(item.quantity) > 10_000
          || !Number.isSafeInteger(item.unit_price_minor) || Number(item.unit_price_minor) < 0) {
        throw new TypeError("purchase item quantity or unit price is invalid");
      }
    }
    return this.request({ type: "create_purchase_intent", request });
  }

  getPurchaseIntent(intentId: string): Promise<PurchaseIntentResult> {
    assertBounded(intentId, "intent_id", 128);
    return this.request({ type: "get_purchase_intent", intent_id: intentId });
  }

  executePurchaseIntent(intentId: string): Promise<PurchaseIntentResult> {
    assertBounded(intentId, "intent_id", 128);
    return this.request({ type: "execute_purchase_intent", intent_id: intentId });
  }

  cancelPurchaseIntent(intentId: string): Promise<PurchaseIntentResult> {
    assertBounded(intentId, "intent_id", 128);
    return this.request({ type: "cancel_purchase_intent", intent_id: intentId });
  }

  listTransactions(cursor: string | null = null, limit = 25): Promise<TransactionPage> {
    if (cursor !== null) assertBounded(cursor, "cursor", 128);
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > 50) {
      throw new TypeError("limit must be an integer between 1 and 50");
    }
    return this.request({ type: "list_transactions_page", cursor, limit });
  }

  getReceipt(intentId: string): Promise<Record<string, unknown>> {
    assertBounded(intentId, "intent_id", 128);
    return this.request({ type: "get_receipt", intent_id: intentId });
  }
}
