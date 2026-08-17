#!/usr/bin/env node
import assert from "node:assert/strict";
import { mkdtemp, chmod, lstat, readFile, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createConnection } from "node:net";
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { chromium } from "playwright-core";

const root = resolve(import.meta.dirname, "..");
const binary = join(root, "target", "debug", "cixa");
function findBrowser() {
  const candidates = [process.env.CIXA_BROWSER_EXECUTABLE, "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium", "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"].filter(Boolean);
  for (const candidate of candidates) if (existsSync(candidate)) return candidate;
  for (const name of ["google-chrome", "chromium", "chromium-browser"]) {
    try { const candidate=execFileSync("which",[name],{encoding:"utf8"}).trim();if(candidate)return candidate; } catch {}
  }
  throw new Error("Chrome or Chromium is required for dashboard UI verification. Set CIXA_BROWSER_EXECUTABLE to its absolute path.");
}
const chrome = findBrowser();
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
  await waitFor(async () => { try { const response=await fetch(`http://127.0.0.1:${port}/`);return response.status===200; } catch { return false; } }, "dashboard did not start");
  browser = await chromium.launch({ headless:true, executablePath:chrome });
  const context = await browser.newContext({ viewport:{width:1440,height:1000}, reducedMotion:"reduce" });
  const page = await context.newPage(); const errors=[];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type()==="error") errors.push(message.text()); });
  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil:"networkidle" });
  await page.getByLabel("Dashboard access token").fill(accessToken);
  await page.getByRole("button", {name:"Unlock console"}).click();
  await assert.doesNotReject(() => page.getByRole("heading", {name:"All quiet."}).waitFor());
  await page.evaluate(() => document.fonts.ready);
  assert.equal((await page.evaluate(() => document.fonts.load("16px Manrope"))).length > 0, true);
  assert.equal((await page.evaluate(() => document.fonts.load("16px Newsreader"))).length > 0, true);
  assert.match(await page.locator("body").evaluate((element) => getComputedStyle(element).fontFamily), /Manrope/);
  assert.match(await page.locator("#today-heading").evaluate((element) => getComputedStyle(element).fontFamily), /Newsreader/);
  assert.equal((await page.getByRole("button", {name:"Refresh"}).textContent()).trim(), "");
  await page.getByText("Spent today", {exact:true}).waitFor();
  await page.getByText("Brought in", {exact:true}).waitFor();
  await page.getByText("Still allowed today", {exact:true}).waitFor();
  await page.getByText("Nothing needs a decision.", {exact:true}).waitFor();
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);

  await page.getByRole("link", {name:"Agents"}).first().click();
  await page.getByRole("button", {name:"Create agent"}).click();
  const dialog = page.locator("#action-dialog");
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
  await depositForm.getByLabel("Amount").fill("8.00");
  await depositForm.getByLabel("Source").fill("Verified browser invoice");
  await depositForm.getByLabel("Provider reference").fill("browser-deposit-2");
  await depositForm.getByLabel("I checked the provider's own record").check();
  await depositForm.getByLabel("Credit to agent").selectOption(agentId);
  await depositForm.getByRole("button", {name:"Record arrival"}).click();
  await dialog.getByRole("button", {name:"Record verified arrival"}).click();
  await dialog.waitFor({state:"hidden"});
  await page.getByRole("link", {name:/Today/}).first().click();
  await page.getByRole("button", {name:"Refresh"}).click();
  await page.getByText("CA$8.00", {exact:true}).waitFor();
  await page.getByText("CA$12.00 more waiting on you", {exact:true}).waitFor();
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
  await page.getByRole("button", {name:"Begin owner handoff"}).click();
  await dialog.getByLabel("I will keep the agent suspended and submit at most once").check();
  await dialog.getByRole("button", {name:"Prepare handoff"}).click();
  await page.getByRole("heading", {name:"Verify before paying"}).waitFor();
  await page.getByText("https://merchant.example.test/checkout", {exact:true}).waitFor();
  await page.getByRole("button", {name:"I submitted exactly once"}).click();
  await dialog.getByRole("button", {name:"Complete handoff"}).click();
  await dialog.waitFor({state:"hidden"});
  await page.getByRole("button", {name:"Close details"}).click();
  await page.getByRole("button", {name:"Waiting"}).click();
  await page.getByRole("button", {name:/Dataset approve/}).click();
  await page.getByRole("button", {name:"Reconcile"}).click();
  await dialog.getByLabel("Provider reference").fill("browser-payment-1");
  await dialog.getByRole("button", {name:"Record outcome"}).click();
  await dialog.waitFor({state:"hidden"});
  await page.getByRole("button", {name:"All", exact:true}).click();
  await page.getByRole("button", {name:/Dataset approve/}).click();
  await page.getByText("Sanitized receipt", {exact:true}).waitFor();
  await page.getByRole("button", {name:"Close details"}).click();
  await page.getByRole("button", {name:"Stopped"}).click();
  await page.locator("#ledger-list").getByText("Cancelled", {exact:true}).waitFor();
  const ownerToken=(await readFile(ownerFile,"utf8")).trim();const historyOverview=await rpc(ownerSocket,ownerToken,{type:"owner_get_dashboard"});const historyPolicy=structuredClone(historyOverview.policies[historyOverview.agents.find((agent)=>agent.id===agentId).policy_id]);historyPolicy.max_transactions_per_minute=100;await rpc(ownerSocket,ownerToken,{type:"owner_update_policy",agent_id:agentId,policy:historyPolicy});
  for (let index=0;index<26;index+=1) await rpc(agentSocket,agentToken,{type:"create_purchase_intent",request:purchase(`history-${index}`,100,"merchant.example.test")});
  await page.getByRole("button", {name:"Refresh"}).click();
  await page.getByRole("button", {name:"All",exact:true}).click();
  await page.getByText("Showing 27 of 28 attempts", {exact:true}).waitFor();
  await page.getByRole("button", {name:"Load older attempts"}).click();
  await page.getByText("Showing 28 of 28 attempts", {exact:true}).waitFor();
  await page.getByRole("button", {name:"Refresh"}).click();
  await page.getByText("Showing 28 of 28 attempts", {exact:true}).waitFor();

  await page.getByRole("link", {name:"Agents"}).first().click();
  await rpc(ownerSocket,ownerToken,{type:"owner_set_emergency_stop",stopped:true});await rpc(ownerSocket,ownerToken,{type:"owner_set_emergency_stop",stopped:false});await page.getByRole("button",{name:"Refresh"}).click();await page.getByRole("button",{name:"Let it spend"}).click();await dialog.getByRole("button",{name:"Let it spend"}).click();await page.getByText("Active",{exact:true}).waitFor();
  await page.getByRole("button", {name:/Open Research runner settings/i}).click();
  await page.getByPlaceholder("merchant.example.test").fill("trusted.example.test");
  await page.getByRole("button", {name:"Trust merchant"}).click();
  await dialog.getByRole("button", {name:"Trust merchant"}).click();
  await dialog.waitFor({state:"hidden"});
  await page.getByRole("button", {name:"Halve today's allowance"}).click();
  await dialog.getByRole("button", {name:"Halve allowance"}).click();
  await page.getByText(/CA\$12\.50/, {exact:false}).first().waitFor();
  await page.getByRole("button", {name:"Pause spending"}).click();
  await page.locator("#agent-list").getByText("Paused", {exact:true}).waitFor();
  await page.getByRole("button", {name:/Open Research runner settings/i}).click();
  await page.getByRole("button", {name:"Edit policy"}).click();
  await dialog.getByLabel("Most per purchase").fill("20.00");
  await dialog.getByRole("button", {name:"Save policy"}).click();
  await page.getByRole("button", {name:/Open Research runner settings/i}).click();
  await page.getByText(/20\.00/, {exact:false}).first().waitFor();
  await page.getByRole("button", {name:"Close details"}).click();

  await page.getByRole("link", {name:"Trust"}).first().click();
  await page.getByRole("button", {name:"Audit"}).click();
  await page.getByText(/Audit chain verified/).waitFor();
  const olderAudit = page.getByRole("button", {name:"Load older audit events"});
  if (await olderAudit.isVisible()) { const firstAuditCount=await page.locator("#audit-list .audit-entry").count();await olderAudit.click();await waitFor(async()=>await page.locator("#audit-list .audit-entry").count()>firstAuditCount,"older audit page did not load");const loadedAuditCount=await page.locator("#audit-list .audit-entry").count();await page.getByRole("button",{name:"Refresh"}).click();assert.equal(await page.locator("#audit-list .audit-entry").count(),loadedAuditCount); }
  await page.getByText("Technical evidence", {exact:true}).first().click();
  assert.equal(await page.locator("#audit-list details").first().getAttribute("open"), "");
  const download = page.waitForEvent("download");
  await page.getByRole("button", {name:"Export recent sanitized audit"}).click();
  assert.match((await download).suggestedFilename(), /cixa-sanitized-export/);
  await page.getByRole("button", {name:"Boundaries"}).click();
  await page.getByText("Unknown means stop and check").waitFor();
  await page.getByRole("button", {name:"Stop all spending"}).click();
  await dialog.getByRole("button", {name:"Stop all spending"}).click();
  await page.locator("#stop-banner").getByText("Spending is stopped.", {exact:true}).waitFor();
  await page.getByRole("button", {name:"Start again"}).click();
  await dialog.getByRole("button", {name:"Start again"}).click();
  await page.locator("#stop-banner").getByText("Spending is stopped.", {exact:true}).waitFor({state:"hidden"});

  await page.getByRole("button", {name:"Use dark theme"}).click();
  assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
  await page.reload({waitUntil:"networkidle"});
  assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
  await page.getByRole("button", {name:"Use light theme"}).click();
  assert.equal(await page.locator("html").getAttribute("data-theme"), "light");
  await page.setViewportSize({width:834,height:1112});
  await page.getByRole("link", {name:"Today"}).last().click();
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  assert.equal(await page.locator("#today-metrics").evaluate((list) => getComputedStyle(list).gridTemplateColumns.split(" ").length), 1);
  await page.evaluate(() => scrollTo(0,0));
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-today-834.png"),fullPage:true});
  await page.setViewportSize({width:390,height:844});
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  await page.locator("#recent-list .ledger-row").first().click();
  const drawerButtons=page.locator("#detail-drawer button:visible");await drawerButtons.last().focus();
  await page.keyboard.press("Tab");
  assert.equal(await page.getByRole("button", {name:"Close details"}).evaluate((element) => element === document.activeElement), true);
  await page.getByRole("button", {name:"Close details"}).click();
  await page.waitForTimeout(4800);
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-mobile.png"),fullPage:true});
  await page.setViewportSize({width:1440,height:1000});
  await page.evaluate(() => scrollTo(0,0));
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console.png"),fullPage:true});
  await page.setViewportSize({width:1024,height:900});
  await page.evaluate(() => scrollTo(0,0));
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  assert.equal(await page.locator("#today-metrics").evaluate((list) => getComputedStyle(list).gridTemplateColumns.split(" ").length), 3);
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-today-1024.png"),fullPage:true});
  await page.setViewportSize({width:1440,height:1000});
  await page.getByRole("link", {name:"Agents"}).first().click();
  await page.evaluate(() => scrollTo(0,0));
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-agents-1440.png"),fullPage:true});
  await page.setViewportSize({width:1024,height:900});
  await page.evaluate(() => scrollTo(0,0));
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  assert.equal(await page.locator("#agent-list").evaluate((list) => getComputedStyle(list).gridTemplateColumns.split(" ").length), 1);
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-agents-1024.png"),fullPage:true});
  await page.setViewportSize({width:834,height:1112});
  await page.evaluate(() => scrollTo(0,0));
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-agents-834.png"),fullPage:true});
  await page.setViewportSize({width:390,height:844});
  await page.evaluate(() => scrollTo(0,0));
  assert.equal(await page.locator("body").evaluate((body) => body.scrollWidth <= body.clientWidth), true);
  await page.screenshot({path:join(root,"build","ui-artifacts","owner-console-agents-390.png"),fullPage:true});
  await page.setViewportSize({width:1440,height:1000});
  await page.getByRole("button", {name:/Open Research runner settings/i}).click();
  await page.getByRole("button", {name:"Revoke capability"}).click();
  await dialog.getByRole("button", {name:"Revoke capability"}).click();
  await page.locator("#agent-list").getByText("Revoked", {exact:true}).waitFor();
  await page.getByRole("button", {name:/Open Research runner settings/i}).click();
  await page.getByRole("button",{name:"Rotate capability"}).click();
  await dialog.getByRole("button",{name:"Cancel",exact:true}).click();
  await dialog.waitFor({state:"hidden"});
  await page.locator("#agent-list").getByText("Revoked",{exact:true}).waitFor();
  await page.getByRole("button", {name:/Open Research runner settings/i}).click();
  await page.getByRole("button",{name:"Rotate capability"}).click();await dialog.getByLabel("New token filename").fill("research-runner-rotated.token");await dialog.getByRole("button",{name:"Rotate capability"}).click();await page.getByText("Active",{exact:true}).waitFor();const rotatedToken=(await readFile(join(directory,"agent-tokens","research-runner-rotated.token"),"utf8")).trim();assert.equal(rotatedToken.length,64);await assert.rejects(()=>rpc(agentSocket,agentToken,{type:"get_status"}));assert.equal((await rpc(agentSocket,rotatedToken,{type:"get_status"})).mode,"approval_required");
  assert.deepEqual(errors, []);
  daemon.kill("SIGTERM");
  await new Promise((resolveExit) => daemon.once("exit",resolveExit));
  await page.getByRole("button", {name:"Refresh"}).click();
  await page.getByText("Cixa is offline", {exact:true}).waitFor();
  await page.getByText(/Financial data and spending status may be stale/).waitFor();
  console.log("owner dashboard browser workflow assertions passed");
} finally {
  if (browser) await browser.close();
  for (const child of [dashboard,daemon]) if (child && !child.killed) { child.kill("SIGTERM"); await new Promise((resolveExit) => child.once("exit",resolveExit)); }
}
