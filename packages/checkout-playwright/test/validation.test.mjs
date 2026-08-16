import assert from "node:assert/strict";
import test from "node:test";
import { observeAndValidate, parseMinorUnits, validateConfiguration } from "../dist/index.js";

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
