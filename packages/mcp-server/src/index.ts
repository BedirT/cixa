import { McpServer } from "@modelcontextprotocol/server";
import { serveStdio } from "@modelcontextprotocol/server/stdio";
import * as z from "zod/v4";
import { BrokerClient, BrokerError } from "agent-treasury-sdk";

const socketPath = process.env.TREASURY_SOCKET_PATH;
const tokenFile = process.env.TREASURY_AGENT_TOKEN_FILE;
if (!socketPath || !tokenFile) {
  console.error("TREASURY_SOCKET_PATH and TREASURY_AGENT_TOKEN_FILE are required; raw tokens are not accepted");
  process.exit(2);
}

const client = new BrokerClient({ socketPath, tokenFile });
const empty = z.object({}).strict();
const idInput = z.object({ intent_id: z.string().min(1).max(128) }).strict();
const money = z.object({
  minor: z.number().int().safe().positive(),
  currency: z.string().regex(/^[A-Z]{3}$/u),
}).strict();
const purchaseInput = z.object({
  idempotency_key: z.string().min(1).max(128),
  amount: money,
  final_total: money,
  merchant_domain: z.string().min(1).max(253),
  category: z.string().min(1).max(64),
  recurring: z.boolean(),
  trial_auto_renew: z.boolean(),
  stored_card: z.boolean(),
  tip_minor: z.number().int().nonnegative().safe(),
  preauthorization: z.boolean(),
  installments: z.boolean(),
  fulfillment_profile: z.string().min(1).max(64),
  payment_form: z.enum(["hosted_fields", "owner_approved_merchant", "merchant_controlled"]),
  redirect_chain: z.array(z.string().min(1).max(4096)).max(8),
  attempts: z.number().int().positive().max(10),
  session_id: z.string().min(1).max(128),
  scenario: z.enum([
    "normal", "decline", "delayed_settlement", "timeout_before_submit", "timeout_after_submit",
    "duplicate_form_submission", "misleading_success_page", "prompt_injection", "amount_changed",
    "currency_changed", "hidden_recurring", "card_saving", "tip", "preauthorization",
    "merchant_controlled_form", "redirect_to_other_domain", "redirect_to_localhost",
    "dns_rebinding_like", "browser_crash",
  ]),
}).strict();

function result(value: unknown) {
  return { content: [{ type: "text" as const, text: JSON.stringify(value) }] };
}

async function safe(call: () => Promise<unknown>) {
  try {
    return result(await call());
  } catch (error) {
    const message = error instanceof BrokerError ? error.message : "broker operation failed";
    return { content: [{ type: "text" as const, text: message }], isError: true };
  }
}

function createServer() {
  const server = new McpServer({ name: "agent-treasury", version: "0.1.0" });
  server.registerTool("treasury_get_status", { description: "Read the agent's sanitized status.", inputSchema: empty }, () => safe(() => client.getStatus()));
  server.registerTool("treasury_get_capabilities", { description: "Read non-sensitive capabilities and immutable owner-only boundaries.", inputSchema: empty }, () => safe(() => client.getCapabilities()));
  server.registerTool("treasury_get_budget", { description: "Read the effective deterministic budget and provider-status labels.", inputSchema: empty }, () => safe(() => client.getBudget()));
  server.registerTool("treasury_get_receive_instructions", { description: "Read public, owner-approved receiving instructions. Notifications are not settlement evidence.", inputSchema: empty }, () => safe(() => client.getReceiveInstructions()));
  server.registerTool("treasury_create_purchase_intent", { description: "Create a bounded purchase intent; policy validation happens outside the agent.", inputSchema: purchaseInput }, (input) => safe(() => client.createPurchaseIntent(input)));
  server.registerTool("treasury_get_purchase_intent", { description: "Read one of this agent's sanitized purchase intents.", inputSchema: idInput }, (input) => safe(() => client.getPurchaseIntent(input.intent_id)));
  server.registerTool("treasury_execute_purchase_intent", { description: "Execute only an intent already authorized for autonomous execution. Ambiguous outcomes cannot be retried.", inputSchema: idInput }, (input) => safe(() => client.executePurchaseIntent(input.intent_id)));
  server.registerTool("treasury_cancel_purchase_intent", { description: "Cancel an unexecuted purchase intent owned by this agent.", inputSchema: idInput }, (input) => safe(() => client.cancelPurchaseIntent(input.intent_id)));
  server.registerTool("treasury_list_transactions", { description: "List this agent's sanitized transactions.", inputSchema: empty }, () => safe(() => client.listTransactions()));
  server.registerTool("treasury_get_receipt", { description: "Read a sanitized receipt with personal information removed.", inputSchema: idInput }, (input) => safe(() => client.getReceipt(input.intent_id)));
  return server;
}

void serveStdio(createServer);
console.error("agent-treasury MCP server running over local stdio");

