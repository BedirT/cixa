#!/usr/bin/env node
import assert from "node:assert/strict";
import { mkdtemp, chmod, lstat, readFile, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createConnection } from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { chromium } from "playwright-core";

const root = resolve(import.meta.dirname, "..");
const binary = join(root, "target", "debug", "cixa");
const chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const directory = await mkdtemp(join(tmpdir(), "cixa-dashboard-ui-"));
const ownerFile = join(directory, "owner.token");
const accessFile = join(directory, "dashboard.token");
const agentSocket = join(directory, "cixa.sock");
const ownerSocket = join(directory, "owner.sock");
const accessToken = "synthetic-browser-owner-secret";
await writeFile(accessFile, `${accessToken}\n`, { mode:0o600 });
await chmod(accessFile, 0o600);
const initialized = spawnSync(binary, ["init", "--data-dir", directory, "--owner-token-file", ownerFile, "--balance-minor", "25000"], { cwd:root, encoding:"utf8" });
assert.equal(initialized.status, 0, initialized.stderr);

const daemon = spawn(binary, ["serve", "--data-dir", directory, "--socket", agentSocket], { cwd:root, stdio:["ignore","pipe","pipe"] });
let dashboard;
let browser;

function waitFor(check, message, timeout = 8000) {
  const started = Date.now();
  return new Promise((resolveWait, reject) => {
    const poll = async () => {
      try { if (await check()) return resolveWait(); } catch {}
      if (Date.now() - started > timeout) return reject(new Error(message));
      setTimeout(poll, 50);
    };
    poll();
  });
}
function rpc(socketPath, token, operation) {
  return new Promise((resolveRpc, reject) => {
    const request = JSON.stringify({ api_version:"v1", request_id:`ui-${Date.now()}-${Math.random()}`, token, operation });
    const channel = createConnection(socketPath); let response = "";
    channel.setEncoding("utf8"); channel.on("connect", () => channel.write(`${request}\n`));
    channel.on("data", (chunk) => { response += chunk; if (response.includes("\n")) channel.end(); });
    channel.on("error", reject); channel.on("end", () => { try { const value=JSON.parse(response.split("\n",1)[0]); value.ok ? resolveRpc(value.data) : reject(new Error(value.error)); } catch(error) { reject(error); } });
  });
}
function purchase(idempotencyKey, amount, merchant) {
  return { idempotency_key:idempotencyKey, amount:{minor:amount,currency:"CAD"}, final_total:{minor:amount,currency:"CAD"}, merchant_domain:merchant, category:"research", items:[{label:`Dataset ${idempotencyKey}`,quantity:1,unit_price_minor:amount}], recurring:false, trial_auto_renew:false, stored_card:false, tip_minor:0, preauthorization:false, installments:false, fulfillment_profile:"digital-email", payment_form:"hosted_fields", redirect_chain:[`https://${merchant}/checkout`], attempts:1, session_id:`session-${idempotencyKey}`, scenario:"normal" };
}

try {
  await waitFor(async () => { try { return (await lstat(ownerSocket)).isSocket(); } catch { return false; } }, "broker sockets did not appear");
  const probe = await import("node:net");
  const port = await new Promise((resolvePort, reject) => { const server=probe.createServer();server.listen(0,"127.0.0.1",()=>{const address=server.address();server.close(()=>resolvePort(address.port));});server.on("error",reject); });
  dashboard = spawn("python3", [join(root,"apps","owner-dashboard","server.py"),"--socket-path",ownerSocket,"--owner-token-file",ownerFile,"--access-token-file",accessFile,"--port",String(port)], { cwd:root, stdio:["ignore","pipe","pipe"] });
  await waitFor(async () => { try { const response=await fetch(`http://127.0.0.1:${port}/`);return response.status===401; } catch { return false; } }, "dashboard did not start");
  browser = await chromium.launch({ headless:true, executablePath:chrome });
  const context = await browser.newContext({ viewport:{width:1440,height:1000}, httpCredentials:{username:"owner",password:accessToken}, reducedMotion:"reduce" });
  const page = await context.newPage(); const errors=[];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type()==="error") errors.push(message.text()); });
  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil:"networkidle" });
  await assert.doesNotReject(() => page.getByRole("heading", {name:"All quiet."}).waitFor());
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);

  await page.getByRole("link", {name:"Agents"}).first().click();
  await page.getByRole("button", {name:"Create agent"}).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Name", {exact:true}).fill("Research Runner");
  await dialog.getByLabel("Token filename", {exact:true}).fill("research-runner.token");
  await dialog.locator("select").selectOption("approval_required");
  await dialog.getByRole("button", {name:"Create agent"}).click();
  await page.getByRole("heading", {name:"Research Runner"}).waitFor();
  const agentToken = (await readFile(join(directory,"agent-tokens","research-runner.token"),"utf8")).trim();
  const overview = await rpc(ownerSocket, (await readFile(ownerFile,"utf8")).trim(), {type:"owner_get_dashboard"});
  const agentId = overview.agents.find((agent) => agent.name === "Research Runner").id;
  await page.getByRole("link", {name:"Trust"}).first().click();
  await page.getByRole("button", {name:"Provider"}).click();
  const providerForm = page.locator("#provider-form");
  await providerForm.getByLabel("Credential reference").fill("keychain://cixa/browser-card");
  await providerForm.getByLabel("Last four").fill("4417");
  await providerForm.getByLabel("Confirmed balance").fill("250.00");
  await providerForm.getByRole("button", {name:"Save provider reference"}).click();
  await dialog.getByRole("button", {name:"Save reference"}).click();
  await dialog.waitFor({state:"hidden"});
  await page.getByRole("button", {name:"Receiving"}).click();
  const receiveForm = page.locator("#receive-form");
  await receiveForm.getByLabel("Public receiving address").fill("public-inbox@example.invalid");
  await receiveForm.getByRole("button", {name:"Save receiving instructions"}).click();
  const depositForm = page.locator("#deposit-form");
  await depositForm.getByLabel("Amount").fill("12.00");
  await depositForm.getByLabel("Source").fill("Browser test invoice");
  await depositForm.getByLabel("Provider reference").fill("browser-deposit-1");
  await depositForm.getByRole("button", {name:"Record arrival"}).click();
  await dialog.getByRole("button", {name:"Keep unverified"}).click();
  await dialog.waitFor({state:"hidden"});
  const first = await rpc(agentSocket, agentToken, {type:"create_purchase_intent",request:purchase("approve",1800,"merchant.example.test")});
  const second = await rpc(agentSocket, agentToken, {type:"create_purchase_intent",request:purchase("deny",2200,"new.example.test")});
  assert.equal(first.state,"approval_required"); assert.equal(second.state,"approval_required");

  await page.getByRole("link", {name:/Today/}).first().click(); await page.getByRole("button", {name:"Refresh"}).click();
  const firstTitle = page.locator("#decision-list").getByText("Dataset approve", {exact:true});
  await firstTitle.waitFor();
  await page.waitForTimeout(4800);
  await mkdir(join(root,"build","ui-artifacts"),{recursive:true});
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-decisions.png"),fullPage:true});
  const firstCard = firstTitle.locator("xpath=ancestor::article");
  await firstCard.getByRole("button", {name:"Allow this once"}).click();
  await dialog.getByRole("button", {name:"Allow once"}).click();
  await firstTitle.waitFor({state:"detached"});
  const secondTitle = page.locator("#decision-list").getByText("Dataset deny", {exact:true});
  const secondCard = secondTitle.locator("xpath=ancestor::article");
  await secondCard.getByRole("button", {name:"Decline"}).click();
  await dialog.getByRole("button", {name:"Decline"}).click();
  await secondTitle.waitFor({state:"detached"});

  await page.getByRole("link", {name:"Ledger"}).first().click();
  await page.getByRole("button", {name:/Dataset approve/}).click();
  await page.getByRole("heading", {name:"$18.00"}).waitFor();
  await page.getByRole("button", {name:"Close details"}).click();
  await page.getByRole("button", {name:"Stopped"}).click();
  await page.locator("#ledger-list").getByText("Cancelled", {exact:true}).waitFor();

  await page.getByRole("link", {name:"Agents"}).first().click();
  await page.getByRole("button", {name:"Pause spending"}).click();
  await page.getByText("Disabled", {exact:true}).waitFor();
  await page.getByRole("button", {name:"Manage limits"}).click();
  await page.getByRole("button", {name:"Edit policy"}).click();
  await dialog.getByLabel("Most per purchase").fill("20.00");
  await dialog.getByRole("button", {name:"Save policy"}).click();
  await page.getByRole("button", {name:"Manage limits"}).click();
  await page.getByText(/20\.00/, {exact:false}).first().waitFor();
  await page.getByRole("button", {name:"Close details"}).click();

  await page.getByRole("link", {name:"Trust"}).first().click();
  await page.getByRole("button", {name:"Audit"}).click();
  await page.getByText(/Audit chain verified/).waitFor();
  await page.getByRole("button", {name:"Boundaries"}).click();
  await page.getByText("Unknown means stop and check").waitFor();
  await page.getByRole("button", {name:"Stop all spending"}).click();
  await dialog.getByRole("button", {name:"Stop all spending"}).click();
  await page.getByText("Spending is stopped.").waitFor();
  await page.getByRole("button", {name:"Start again"}).click();
  await dialog.getByRole("button", {name:"Start again"}).click();
  await page.getByText("Spending is stopped.").waitFor({state:"hidden"});

  await page.getByRole("button", {name:"Use dark theme"}).click();
  assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
  await page.reload({waitUntil:"networkidle"});
  assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
  await page.getByRole("button", {name:"Use light theme"}).click();
  assert.equal(await page.locator("html").getAttribute("data-theme"), "light");
  await page.setViewportSize({width:834,height:1112});
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  await page.setViewportSize({width:390,height:844});
  await page.getByRole("link", {name:"Today"}).last().click();
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  await page.waitForTimeout(4800);
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-mobile.png"),fullPage:true});
  await page.setViewportSize({width:1440,height:1000});
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console.png"),fullPage:true});
  assert.deepEqual(errors, []);
  console.log("owner dashboard browser workflow assertions passed");
} finally {
  if (browser) await browser.close();
  for (const child of [dashboard,daemon]) if (child && !child.killed) { child.kill("SIGTERM"); await new Promise((resolveExit) => child.once("exit",resolveExit)); }
}
