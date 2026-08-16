#!/usr/bin/env node
import { promises as dns } from "node:dns";
import { readFile } from "node:fs/promises";
import { isIP } from "node:net";
import { createInterface } from "node:readline";
import { chromium, type Browser, type BrowserContext, type Page } from "playwright-core";

type Money = { minor: number; currency: string };
type PurchaseRequest = {
  final_total: Money;
  merchant_domain: string;
  items: Array<{ label: string; quantity: number; unit_price_minor: number }>;
  recurring: boolean;
  trial_auto_renew: boolean;
  stored_card: boolean;
  tip_minor: number;
  preauthorization: boolean;
  installments: boolean;
  fulfillment_profile: string;
  payment_form: string;
  redirect_chain: string[];
};
type Secret = { pan: string; expiry: string; cvv: string; cardholder?: string };
type AdapterInput = { request: PurchaseRequest; secret: Secret };
type SelectorConfig = {
  finalTotal: string;
  currency: string;
  fulfillment: string;
  items: string;
  recurring: string;
  trialAutoRenew: string;
  storedCard: string;
  tipMinor: string;
  preauthorization: string;
  installments: string;
  paymentFrame: string;
  pan: string;
  expiry: string;
  cvv: string;
  cardholder?: string;
  submit: string;
};
export type AdapterConfig = {
  browserExecutable: string;
  checkoutUrl: string;
  allowedNavigationOrigins: string[];
  allowedProcessorOrigins: string[];
  selectors: SelectorConfig;
  timeoutMs: number;
};

function fail(message: string): never {
  throw new Error(message);
}

function parseBoolean(value: string): boolean {
  if (value === "true") return true;
  if (value === "false") return false;
  return fail("checkout boolean evidence is not canonical");
}

export function parseMinorUnits(value: string): number {
  if (!/^[0-9]+(?:\.[0-9]{2})$/.test(value)) fail("checkout total is not canonical");
  const [major, minor] = value.split(".");
  const result = Number(major) * 100 + Number(minor);
  if (!Number.isSafeInteger(result) || result <= 0) fail("checkout total is invalid");
  return result;
}

function privateAddress(address: string): boolean {
  if (isIP(address) === 4) {
    const [a, b, c] = address.split(".").map(Number);
    return a === 0 || a === 10 || a === 127 || a >= 224 ||
      (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) ||
      (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) ||
      (a === 192 && b === 0 && (c === 0 || c === 2)) ||
      (a === 198 && (b === 18 || b === 19 || (b === 51 && c === 100))) ||
      (a === 203 && b === 0 && c === 113);
  }
  const normalized = address.toLowerCase();
  if (normalized.startsWith("::ffff:")) return privateAddress(normalized.slice(7));
  return normalized === "::" || normalized === "::1" || normalized.startsWith("fc") ||
    normalized.startsWith("fd") || /^fe[89ab]/.test(normalized) ||
    normalized.startsWith("ff") || normalized.startsWith("2001:db8:");
}

async function resolvePublicHost(hostname: string): Promise<string> {
  const addresses = await dns.lookup(hostname, { all: true, verbatim: true });
  if (addresses.length === 0 || addresses.some(({ address }) => privateAddress(address))) {
    fail("checkout network destination is not public");
  }
  return addresses.find(({ family }) => family === 4)?.address ?? addresses[0].address;
}

function canonicalOrigin(raw: string): string {
  const url = new URL(raw);
  if (url.protocol !== "https:" || url.username || url.password || url.port) {
    fail("checkout origins must be credential-free HTTPS default-port URLs");
  }
  return url.origin;
}

export function validateConfiguration(config: AdapterConfig, request: PurchaseRequest): void {
  if (request.payment_form !== "hosted_fields") fail("only trusted hosted fields are supported");
  const checkout = new URL(config.checkoutUrl);
  if (canonicalOrigin(config.checkoutUrl) !== `https://${request.merchant_domain}`) {
    fail("checkout URL is not bound to the approved merchant");
  }
  if (!config.browserExecutable.startsWith("/")) fail("browser executable must be absolute");
  if (!Number.isInteger(config.timeoutMs) || config.timeoutMs < 1_000 || config.timeoutMs > 120_000) {
    fail("adapter timeout is outside 1..120 seconds");
  }
  const navigation = new Set(config.allowedNavigationOrigins.map(canonicalOrigin));
  navigation.add(checkout.origin);
  for (const redirect of request.redirect_chain) {
    if (!navigation.has(canonicalOrigin(redirect))) fail("approved redirect is outside adapter origins");
  }
  if (config.allowedProcessorOrigins.length === 0) fail("a hosted-fields processor origin is required");
  const processors = config.allowedProcessorOrigins.map(canonicalOrigin);
  if (processors.some((origin) => navigation.has(origin))) {
    fail("hosted-field processor origins must be disjoint from navigation origins");
  }
}

async function text(page: Page, selector: string): Promise<string> {
  const matches = page.locator(selector);
  const count = await matches.count();
  const visible = [];
  for (let index = 0; index < count; index += 1) {
    const candidate = matches.nth(index);
    if (await candidate.isVisible()) visible.push(candidate);
  }
  if (visible.length !== 1) fail("checkout evidence must have exactly one visible match");
  return (await visible[0].textContent())?.trim() ?? fail("checkout evidence is missing");
}

export async function observeAndValidate(page: Page, config: AdapterConfig, request: PurchaseRequest): Promise<void> {
  const selectors = config.selectors;
  if (new URL(page.url()).hostname !== request.merchant_domain) fail("final merchant origin changed");
  if (parseMinorUnits(await text(page, selectors.finalTotal)) !== request.final_total.minor) {
    fail("visible final total differs from the approved total");
  }
  if ((await text(page, selectors.currency)).toUpperCase() !== request.final_total.currency) {
    fail("visible currency differs from the approved currency");
  }
  if (await text(page, selectors.fulfillment) !== request.fulfillment_profile) {
    fail("visible fulfillment differs from the approved profile");
  }
  let observedItems: unknown;
  try {
    observedItems = JSON.parse(await text(page, selectors.items));
  } catch {
    fail("visible line-item evidence is not canonical JSON");
  }
  if (JSON.stringify(observedItems) !== JSON.stringify(request.items)) {
    fail("visible line items differ from the approved items");
  }
  for (const [selector, expected] of [
    [selectors.recurring, request.recurring],
    [selectors.trialAutoRenew, request.trial_auto_renew],
    [selectors.storedCard, request.stored_card],
  ] as const) {
    if (parseBoolean(await text(page, selector)) !== expected) fail("checkout consent facts changed");
  }
  if (parseMinorUnitsAllowZero(await text(page, selectors.tipMinor)) !== request.tip_minor) {
    fail("visible tip differs from the approved tip");
  }
  if (parseBoolean(await text(page, selectors.preauthorization)) !== request.preauthorization) {
    fail("checkout preauthorization fact changed");
  }
  if (parseBoolean(await text(page, selectors.installments)) !== request.installments) {
    fail("checkout installment fact changed");
  }
}

function parseMinorUnitsAllowZero(value: string): number {
  if (!/^[0-9]+(?:\.[0-9]{2})$/.test(value)) fail("checkout amount is not canonical");
  const [major, minor] = value.split(".");
  const result = Number(major) * 100 + Number(minor);
  if (!Number.isSafeInteger(result) || result < 0) fail("checkout amount is invalid");
  return result;
}

async function requireProcessorFrame(
  page: Page,
  config: AdapterConfig,
  processorOrigins: ReadonlySet<string>,
) {
  const frameHandle = await page.locator(config.selectors.paymentFrame).elementHandle();
  const paymentFrame = await frameHandle?.contentFrame();
  if (!paymentFrame
      || canonicalOrigin(paymentFrame.url()) === canonicalOrigin(page.url())
      || !processorOrigins.has(canonicalOrigin(paymentFrame.url()))) {
    fail("payment frame is not cross-origin and owned by an approved processor");
  }
  return paymentFrame;
}

async function run(config: AdapterConfig, input: AdapterInput): Promise<object> {
  validateConfiguration(config, input.request);
  const navigationOrigins = new Set(config.allowedNavigationOrigins.map(canonicalOrigin));
  navigationOrigins.add(canonicalOrigin(config.checkoutUrl));
  const processorOrigins = new Set(config.allowedProcessorOrigins.map(canonicalOrigin));
  const allowedHosts = new Set(
    [...navigationOrigins, ...processorOrigins].map((origin) => new URL(origin).hostname),
  );
  const resolverRules: string[] = [];
  for (const host of allowedHosts) {
    resolverRules.push(`MAP ${host} ${await resolvePublicHost(host)}`);
  }
  resolverRules.push("EXCLUDE localhost");
  let browser: Browser | undefined;
  let context: BrowserContext | undefined;
  try {
    browser = await chromium.launch({
      executablePath: config.browserExecutable,
      headless: true,
      args: [
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        `--host-resolver-rules=${resolverRules.join(",")}`,
      ],
    });
    context = await browser.newContext({
      acceptDownloads: false,
      serviceWorkers: "block",
      permissions: [],
    });
    let page: Page;
    await context.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (url.protocol !== "https:" || url.username || url.password) return route.abort("blockedbyclient");
      const origin = canonicalOrigin(url.toString());
      if (!navigationOrigins.has(origin) && !processorOrigins.has(origin)) return route.abort("blockedbyclient");
      const request = route.request();
      if (request.isNavigationRequest()
          && request.frame().parentFrame() !== null
          && !processorOrigins.has(origin)) {
        return route.abort("blockedbyclient");
      }
      try {
        await resolvePublicHost(url.hostname);
        await route.continue();
      } catch {
        await route.abort("blockedbyclient");
      }
    });
    page = await context.newPage();
    const observedNavigations: string[] = [];
    page.on("request", (requestEvent) => {
      if (requestEvent.isNavigationRequest() && requestEvent.frame() === page.mainFrame()) {
        observedNavigations.push(new URL(requestEvent.url()).toString());
      }
    });
    page.on("download", (download) => void download.cancel());
    page.on("dialog", (dialog) => void dialog.dismiss());
    await page.goto(config.checkoutUrl, { waitUntil: "domcontentloaded", timeout: config.timeoutMs });
    const expectedNavigations = input.request.redirect_chain.map((url) => new URL(url).toString());
    if (JSON.stringify(observedNavigations) !== JSON.stringify(expectedNavigations)) {
      fail("observed redirect chain differs from the approved chain");
    }
    await observeAndValidate(page, config, input.request);
    let paymentFrame = await requireProcessorFrame(page, config, processorOrigins);
    await paymentFrame.locator(config.selectors.pan).fill(input.secret.pan);
    paymentFrame = await requireProcessorFrame(page, config, processorOrigins);
    await paymentFrame.locator(config.selectors.expiry).fill(input.secret.expiry);
    paymentFrame = await requireProcessorFrame(page, config, processorOrigins);
    await paymentFrame.locator(config.selectors.cvv).fill(input.secret.cvv);
    if (config.selectors.cardholder && input.secret.cardholder) {
      paymentFrame = await requireProcessorFrame(page, config, processorOrigins);
      await paymentFrame.locator(config.selectors.cardholder).fill(input.secret.cardholder);
    }
    await observeAndValidate(page, config, input.request);
    await requireProcessorFrame(page, config, processorOrigins);
    if (JSON.stringify(observedNavigations) !== JSON.stringify(expectedNavigations)) {
      fail("checkout navigation changed during the payment critical section");
    }
    await page.locator(config.selectors.submit).click({ noWaitAfter: true });
    return {
      outcome: "unknown",
      reason: "merchant DOM is not authenticated provider evidence; owner reconciliation required",
    };
  } finally {
    input.secret.pan = "";
    input.secret.expiry = "";
    input.secret.cvv = "";
    input.secret.cardholder = "";
    await context?.clearCookies().catch(() => undefined);
    await context?.close().catch(() => undefined);
    await browser?.close().catch(() => undefined);
  }
}

async function main(): Promise<void> {
  const configPath = process.argv[2];
  if (!configPath) fail("adapter config path is required");
  const config = JSON.parse(await readFile(configPath, "utf8")) as AdapterConfig;
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity, terminal: false });
  for await (const line of lines) {
    if (line.length > 16_384) fail("adapter input is too large");
    const input = JSON.parse(line) as AdapterInput;
    process.stdout.write(`${JSON.stringify(await run(config, input))}\n`);
    return;
  }
  fail("adapter input is missing");
}

if (process.argv[1]?.endsWith("index.js")) {
  main().catch((error: unknown) => {
    const message = error instanceof Error ? error.message : "adapter failed";
    process.stderr.write(`checkout adapter failed: ${message.replace(/[\r\n]/g, " ")}\n`);
    process.exitCode = 1;
  });
}
