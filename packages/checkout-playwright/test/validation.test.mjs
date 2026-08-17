import assert from "node:assert/strict";
import test from "node:test";
import {
  observeAndValidate,
  parseMinorUnits,
  privateAddress,
  installBrowserNetworkGuards,
  processorFrameAllowed,
  requestOriginAllowed,
  validateConfiguration,
} from "../dist/index.js";

test("blocks WebSockets and installs pre-page peer-network guards", async () => {
  let websocketPattern;
  let websocketClose;
  let initScript;
  await installBrowserNetworkGuards({
    routeWebSocket: async (pattern, handler) => {
      websocketPattern = pattern;
      await handler({ close: async (options) => { websocketClose = options; } });
    },
    addInitScript: async (script) => { initScript = script; },
  });
  assert.equal(websocketPattern, "**/*");
  assert.equal(websocketClose.code, 1008);
  assert.match(initScript.toString(), /RTCPeerConnection/);
  assert.match(initScript.toString(), /WebTransport/);
});

test("rejects private IPv4-mapped and canonical IPv6 destinations", () => {
  for (const address of [
    "::ffff:7f00:1",
    "::ffff:a9fe:1",
    "::ffff:c0a8:101",
    "::ffff:c000:200",
    "0:0:0:0:0:0:0:1",
    "fe80::1",
    "fec0::1",
    "100::1",
    "64:ff9b:1::c0a8:101",
    "2001:20::1",
    "2001:db8::1",
    "2002:c0a8:101::",
    "3fff::1",
  ]) {
    assert.equal(privateAddress(address), true, address);
  }
  assert.equal(privateAddress("2606:4700:4700::1111"), false);
});

const request = {
  final_total: { minor: 1234, currency: "CAD" },
  merchant_domain: "merchant.example.com",
  items: [{ label: "test item", quantity: 1, unit_price_minor: 1234 }],
  recurring: false,
  trial_auto_renew: false,
  stored_card: false,
  tip_minor: 0,
  preauthorization: false,
  installments: false,
  fulfillment_profile: "digital-email",
  payment_form: "hosted_fields",
  redirect_chain: ["https://merchant.example.com/checkout"],
};
const config = {
  browserExecutable: "/owned/browser",
  checkoutUrl: "https://merchant.example.com/checkout",
  allowedNavigationOrigins: ["https://merchant.example.com"],
  allowedProcessorOrigins: ["https://processor.example.com"],
  timeoutMs: 30_000,
  selectors: {},
};

test("validates canonical money and bound origins", () => {
  assert.equal(parseMinorUnits("12.34"), 1234);
  assert.doesNotThrow(() => validateConfiguration(config, request));
  assert.throws(() => parseMinorUnits("12.3"));
  assert.throws(() => validateConfiguration({ ...config, checkoutUrl: "https://evil.example/" }, request));
  assert.throws(() => validateConfiguration(config, { ...request, payment_form: "merchant_controlled" }));
  assert.throws(() => validateConfiguration({
    ...config,
    allowedProcessorOrigins: ["https://merchant.example.com"],
  }, request));
});

test("route and frame policy deny merchant subframes and origin changes", () => {
  const navigation = new Set(["https://merchant.example.com"]);
  const processors = new Set(["https://processor.example.com"]);
  assert.equal(requestOriginAllowed(
    "https://merchant.example.com/checkout", navigation, processors, false,
  ), true);
  assert.equal(requestOriginAllowed(
    "https://merchant.example.com/embedded", navigation, processors, true,
  ), false);
  assert.equal(requestOriginAllowed(
    "https://processor.example.com/fields", navigation, processors, true,
  ), true);
  assert.equal(requestOriginAllowed(
    "http://processor.example.com/fields", navigation, processors, true,
  ), false);
  assert.equal(processorFrameAllowed(
    "https://processor.example.com/fields", "https://merchant.example.com/checkout", processors,
  ), true);
  assert.equal(processorFrameAllowed(
    "https://merchant.example.com/fields", "https://merchant.example.com/checkout", processors,
  ), false);
  assert.equal(processorFrameAllowed(
    "https://other-processor.example.com/fields", "https://merchant.example.com/checkout", processors,
  ), false);
});

test("live fact mutation fails the pre-submit revalidation", async () => {
  const values = {
    total: "12.34",
    currency: "CAD",
    fulfillment: "digital-email",
    items: JSON.stringify(request.items),
    recurring: "false",
    trial: "false",
    stored: "false",
    tip: "0.00",
    preauthorization: "false",
    installments: "false",
  };
  const selectors = {
    finalTotal: "total",
    currency: "currency",
    fulfillment: "fulfillment",
    items: "items",
    recurring: "recurring",
    trialAutoRenew: "trial",
    storedCard: "stored",
    tipMinor: "tip",
    preauthorization: "preauthorization",
    installments: "installments",
  };
  const locator = (selector) => ({
    count: async () => 1,
    nth: () => ({ isVisible: async () => true, textContent: async () => values[selector] }),
  });
  const page = {
    url: () => "https://merchant.example.com/checkout",
    locator,
  };
  await assert.doesNotReject(() => observeAndValidate(page, { ...config, selectors }, request));
  values.recurring = "true";
  await assert.rejects(() => observeAndValidate(page, { ...config, selectors }, request));
});

test("hidden decoy evidence is rejected", async () => {
  const locator = () => ({
    count: async () => 2,
    nth: (index) => ({
      isVisible: async () => index === 1,
      textContent: async () => index === 0 ? "12.34" : "99.99",
    }),
  });
  const selectors = {
    finalTotal: "total", currency: "currency", fulfillment: "fulfillment", items: "items",
    recurring: "recurring", trialAutoRenew: "trial", storedCard: "stored", tipMinor: "tip",
    preauthorization: "preauthorization", installments: "installments",
  };
  await assert.rejects(() => observeAndValidate({
    url: () => "https://merchant.example.com/checkout", locator,
  }, { ...config, selectors }, request));
});
