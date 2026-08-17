"use strict";

let csrf = "";
const state = { overview: null, audit: null, ledgerTransactions:[], ledgerCursor:null, ledgerTotal:0, auditCursor:null, route: "today", ledgerFilter: "all", search: "", busy: false, dialogAction: null, lastFocus: null, lastSuccessfulRefresh: null, offline:false };
const waitingStates = new Set(["approval_required", "provider_pending", "unknown", "reconciliation_required"]);
const paidStates = new Set(["settled", "refunded"]);
const stoppedStates = new Set(["declined", "failed", "cancelled"]);

function $(selector) { return document.querySelector(selector); }
function $$(selector) { return [...document.querySelectorAll(selector)]; }
function node(tag, options = {}, children = []) {
  const value = document.createElement(tag);
  for (const [key, item] of Object.entries(options)) {
    if (key === "class") value.className = item;
    else if (key === "text") value.textContent = item;
    else if (key === "dataset") Object.assign(value.dataset, item);
    else if (key === "attrs") for (const [name, attribute] of Object.entries(item)) value.setAttribute(name, attribute);
    else if (key.startsWith("on")) value.addEventListener(key.slice(2).toLowerCase(), item);
    else value[key] = item;
  }
  for (const child of [].concat(children)) if (child != null) value.append(child instanceof Node ? child : document.createTextNode(String(child)));
  return value;
}
function svgIcon(symbolId) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24"); svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#${symbolId}`); svg.append(use); return svg;
}
function replace(target, children) { target.replaceChildren(...[].concat(children)); }
function title(value) { return String(value ?? "unknown").replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase()); }
function money(value) {
  if (!value || !Number.isSafeInteger(value.minor) || typeof value.currency !== "string") return "Unavailable";
  try { return new Intl.NumberFormat(undefined, { style: "currency", currency: value.currency }).format(value.minor / 100); }
  catch { return `${(value.minor / 100).toFixed(2)} ${value.currency}`; }
}
function when(epoch) { return Number.isFinite(epoch) ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(epoch * 1000)) : "Time unavailable"; }
function durationMinutes(seconds) { return `${Math.max(1,Math.ceil(seconds/60))} ${Math.ceil(seconds/60)===1?"minute":"minutes"}`; }
function agentName(id) { return state.overview?.agents.find((agent) => agent.id === id)?.name ?? id; }
function purchaseTitle(intent) { return intent.items?.map((item) => item.label).filter(Boolean).join(", ") || intent.category || "Purchase"; }
function statusTone(value) { return paidStates.has(value) ? "success" : stoppedStates.has(value) ? "danger" : waitingStates.has(value) ? "waiting" : ""; }
function statusCopy(value) {
  return ({ approval_required:"Needs you", provider_pending:"Provider pending", unknown:"Not confirmed", reconciliation_required:"Needs reconciliation", settled:"Paid", refunded:"Refunded", declined:"Declined", failed:"Stopped", cancelled:"Cancelled", executing:"Paying now", funds_reserved:"Funds held", approved:"Allowed once", policy_validated:"Policy checked", proposed:"Checking" })[value] ?? title(value);
}
function humanReason(value) {
  const reason = String(value ?? "");
  if (reason.includes("manual prepaid-card checkout requires an authenticated owner handoff")) return "This manual checkout needs you to take over before payment is sent.";
  if (reason === "owner approval is required") return "This purchase needs your approval.";
  if (reason === "owner_denied") return "You declined this purchase.";
  return reason.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());
}

async function api(path, options = {}) {
  const response = await fetch(path, { credentials:"same-origin", ...options, headers:{ ...(options.body ? { "Content-Type":"application/json", "X-CSRF-Token":csrf } : {}), ...(options.headers ?? {}) } });
  let value;
  try { value = await response.json(); } catch { throw new Error(`Cixa returned an unreadable response (${response.status}).`); }
  if (!response.ok) {
    const error = new Error(value.error ?? `Request failed (${response.status}).`);
    error.status = response.status; error.details = value;
    throw error;
  }
  return value;
}
function readCsrf() { csrf = document.cookie.split("; ").find((part) => part.startsWith("csrf="))?.split("=")[1] ?? ""; }
readCsrf();
async function loadLedger(reset = false) {
  const cursor = reset ? null : state.ledgerCursor;
  const page = await api(`/api/transactions?limit=25${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`);
  state.ledgerTransactions = reset ? page.transactions : [...new Map([...state.ledgerTransactions,...page.transactions].map((intent)=>[intent.id,intent])).values()];
  state.ledgerCursor = page.next_cursor; state.ledgerTotal = page.transactions_total; renderLedger();
}
async function refreshLedger() {
  const page = await api("/api/transactions?limit=25");
  const byId = new Map([...state.ledgerTransactions, ...page.transactions].map((intent)=>[intent.id,intent]));
  state.ledgerTransactions = [...byId.values()]; state.ledgerTotal = page.transactions_total;
  if (state.ledgerTransactions.length >= state.ledgerTotal) state.ledgerCursor = null;
  else if (!state.ledgerCursor) state.ledgerCursor = page.next_cursor;
  renderLedger();
}
async function loadAudit(reset = false) {
  const cursor = reset ? null : state.auditCursor;
  const page = await api(`/api/audit?limit=25${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`);
  state.audit = reset ? page : {...page, entries:[...new Map([...(state.audit?.entries ?? []),...page.entries].map((entry)=>[entry.sequence,entry])).values()].sort((a,b)=>b.sequence-a.sequence)};
  state.auditCursor = page.next_cursor; renderTrust();
}
async function refreshAudit() {
  const page = await api("/api/audit?limit=25");
  const bySequence = new Map([...(state.audit?.entries ?? []), ...page.entries].map((entry)=>[entry.sequence,entry]));
  state.audit = {...page,entries:[...bySequence.values()].sort((a,b)=>b.sequence-a.sequence)};
  if (state.audit.entries.length >= page.entries_total) state.auditCursor = null;
  else if (!state.auditCursor) state.auditCursor = page.next_cursor;
  renderTrust();
}
async function post(path, body, success) {
  if (state.busy) return;
  state.busy = true; document.body.classList.add("busy"); $("main").setAttribute("aria-busy", "true");
  try { const value = await api(path, { method:"POST", body:JSON.stringify(body) }); toast(success); await refresh(); return value; }
  catch (error) {
    if (error.details?.activation_uncertain) await refresh().catch(() => {});
    toast(error.message, true); throw error;
  }
  finally { state.busy = false; document.body.classList.remove("busy"); $("main").removeAttribute("aria-busy"); }
}
function toast(message, error = false) {
  const item = node("div", { class:`toast${error ? " error" : ""}`, text:message, attrs:{ role:"status" } });
  $("#toast-region").append(item); $("#operation-status").textContent = message;
  window.setTimeout(() => item.remove(), 4500);
}

async function refresh() {
  try {
    const initial = !state.overview;
    state.overview = await api("/api/overview");
    if (initial) await loadLedger(true); else await refreshLedger();
    state.lastSuccessfulRefresh = new Date(); state.offline = false; render();
    try { if (initial) await loadAudit(true); else await refreshAudit(); }
    catch { state.audit = null; renderTrust(); }
  } catch (error) {
    state.offline = true; renderChrome(); throw error;
  }
}
function render() {
  renderChrome(); renderToday(); renderLedger(); renderAgents(); renderTrust();
}
function renderChrome() {
  if (!state.overview) return;
  const data = state.overview; const count = (data.pending_approvals_total ?? data.pending_approvals.length) + (data.reconciliation_required_total ?? data.reconciliation_required.length);
  $("#connection-label").textContent = state.offline ? "Cixa is offline" : data.emergency_stop ? "Spending stopped" : "Watching";
  $("#watch-copy").textContent = state.offline ? `Showing the last update from ${state.lastSuccessfulRefresh?.toLocaleTimeString() ?? "an unknown time"}.` : data.emergency_stop ? "No new purchase can begin." : `Every checkout is read before money moves. Updated ${state.lastSuccessfulRefresh?.toLocaleTimeString([], {hour:"2-digit",minute:"2-digit",second:"2-digit"})}.`;
  $(".status-dot").classList.toggle("offline", state.offline);
  $("#decision-count").hidden = count === 0; $("#decision-count").textContent = String(count);
  $("#emergency-stop-button").hidden = data.emergency_stop; $("#stop-banner").hidden = !data.emergency_stop;
  const warnings = data.unsafe_modes.map((mode) => mode.includes("manual prepaid-card") ? "Manual card mode is on. You will finish each checkout and confirm what happened." : humanReason(mode));
  if (data.provider.balance_evidence?.stale) warnings.push(`The provider balance expired ${when(data.provider.balance_evidence.expires_at)}. Refresh it before relying on available funds.`);
  if (data.transactions_truncated || data.pending_approvals_truncated || data.reconciliation_required_truncated) warnings.push("This view shows the newest 10 records. Older history remains in Cixa's local ledger.");
  if (state.offline) warnings.unshift("The local broker did not answer. Financial data and spending status may be stale.");
  $("#unsafe-banner").hidden = warnings.length === 0; $("#unsafe-banner").textContent = warnings.join(" ");
  const now = new Date(); $("#current-time").dateTime = now.toISOString(); $("#current-time").textContent = now.toLocaleString(undefined, { weekday:"short", month:"short", day:"numeric", hour:"2-digit", minute:"2-digit" });
}
function metric(label, value, note, percent, tone = "") {
  return node("article", { class:`metric-card ${tone ? `tone-${tone}` : ""}` }, [node("div", { class:"metric-label", text:label }), node("div", { class:"metric-value", text:value }), node("progress", { class:"metric-track", value:Math.max(0, Math.min(100, percent)), max:100, attrs:{ "aria-label":`${label}: ${Math.round(percent)} percent` } }), node("p", { class:"metric-note", text:note })]);
}
function renderToday() {
  const data = state.overview; const pending = [...data.pending_approvals, ...data.reconciliation_required.filter((item) => !data.pending_approvals.some((pendingItem) => pendingItem.id === item.id))];
  const pendingCount = (data.pending_approvals_total ?? data.pending_approvals.length) + (data.reconciliation_required_total ?? data.reconciliation_required.length);
  $("#today-date").textContent = new Date().toLocaleDateString(undefined, { weekday:"long", month:"long", day:"numeric" });
  $("#today-heading").textContent = pendingCount ? "A few things need you." : "All quiet.";
  $("#today-summary").textContent = pendingCount ? `${pendingCount} ${pendingCount === 1 ? "decision is" : "decisions are"} waiting. Everything else stays inside your rules.` : "Nothing needs a decision. Cixa is reading every checkout as it comes in.";
  const currency = data.provider.balance?.currency ?? "CAD";
  const currencyAgents=data.agents.filter((agent)=>agent.budget?.usage?.rolling_24h_amount?.currency===currency);const active = currencyAgents.filter((agent) => !agent.revoked && ["approval_required","bounded_autonomous"].includes(agent.mode) && agent.expires_at > Date.now()/1000 && agent.broker_session_expires_at > Date.now()/1000);
  const spent = currencyAgents.reduce((sum, agent) => sum + agent.budget.usage.rolling_24h_amount.minor, 0); const policyRemaining = active.reduce((sum, agent) => sum + Math.min(agent.budget.remaining_rolling_24h.minor,agent.budget.remaining_session.minor,agent.budget.remaining_lifetime.minor), 0); const providerRemaining = data.provider.balance_evidence?.stale ? 0 : data.provider.balance?.minor ?? 0; const authorityRemaining = data.available_authority?.currency === currency ? data.available_authority.minor : 0; const remaining = Math.max(0, Math.min(policyRemaining, providerRemaining, authorityRemaining)); const allowance = spent + remaining;
  const evidence = data.provider.balance_evidence?.stale ? "Expired evidence" : title(data.provider.balance_status);
  replace($("#today-metrics"), [metric("Used or held in 24 hours", money({ minor:spent, currency }), "Settled spending plus funds held for unresolved outcomes", allowance ? spent / allowance * 100 : 0), metric("Provider reports", money(data.provider.balance), evidence, data.provider.balance_evidence?.stale ? 0 : 100, data.provider.balance_evidence?.stale ? "" : "green"), metric("Available now", money({ minor:remaining, currency }), "Capped by shared funds, ledger authority, and active agent limits", allowance ? remaining / allowance * 100 : 0)]);
  $("#waiting-count").textContent = `${pendingCount} ${pendingCount === 1 ? "item" : "items"}`; replace($("#decision-list"), pending.length ? pending.map(decisionCard) : empty("Nothing needs you right now."));
  const recent = [...data.transactions].sort((a,b) => b.updated_at - a.updated_at).slice(0,4); replace($("#recent-list"), recent.length ? recent.map(ledgerRow) : empty("No purchases have been attempted yet."));
}
function empty(message) { return node("div", { class:"empty-state", text:message }); }
function stateBadge(intent) { return node("span", { class:`state-badge ${statusTone(intent.state)}`, text:statusCopy(intent.state) }); }
function decisionCard(intent) {
  const approval = intent.state === "approval_required"; const reconcile = waitingStates.has(intent.state) && !approval;
  const reasons = intent.decision?.reasons?.length ? intent.decision.reasons.map(humanReason).join(" ") : approval ? "The request needs a one-time owner decision before it can proceed." : "Cixa cannot safely determine the provider outcome. Check the provider's own record before resolving it.";
  const facts = [intent.checkout_facts?.recurring ? "Recurring charge" : "One-off charge", `${title(intent.checkout_facts?.payment_form)} form`, ...(intent.decision?.reasons ?? []).slice(0,2).map(humanReason)].map((value) => node("span", { class:"chip", text:value }));
  const actions = [];
  if (approval) {
    actions.push(
      button("Allow this once", "primary-button", () => confirmAction({ title:"Allow this one purchase?", copy:"Cixa will re-check the request before execution. This does not trust the merchant for future purchases.", facts:[["Amount",money(intent.amount)],["Shop",intent.merchant_domain],["Agent",agentName(intent.agent_id)]], label:"Allow once", action:() => post("/api/approvals/approve", { intent_id:intent.id }, "Allowed once. The merchant is not permanently trusted.") })),
      button("Decline", "secondary-button", () => confirmAction({ title:"Decline this purchase?", copy:"The intent will be cancelled and cannot be restarted.", label:"Decline", danger:true, action:() => post("/api/approvals/deny", { intent_id:intent.id }, "Declined. Nothing was spent.") })),
    );
  }
  if (reconcile) actions.push(button("I checked with the provider", "primary-button", () => openReconcile(intent)));
  actions.push(button("Everything Cixa checked", "quiet-button details-link", () => openIntent(intent)));
  return node("article", { class:"decision-card" }, [node("div", { class:"decision-top" }, [stateBadge(intent), node("time", { text:when(intent.created_at) })]), node("div", { class:"decision-body" }, [node("div", {}, [node("div", { class:"decision-amount", text:money(intent.amount) }), node("div", { class:"decision-title", text:purchaseTitle(intent) }), node("div", { class:"decision-meta", text:`${agentName(intent.agent_id)} · ${intent.merchant_domain}` })]), node("div", {}, [node("p", { class:"decision-explanation", text:reasons }), node("div", { class:"chip-row" }, facts)])]), node("div", { class:"card-actions" }, actions)]);
}
function button(label, className, action) { return node("button", { type:"button", class:className, text:label, onclick:action }); }
function ledgerRow(intent) {
  const reason = stoppedStates.has(intent.state) || waitingStates.has(intent.state) ? intent.last_error || intent.decision?.reasons?.[0] : null;
  return node("button", { type:"button", class:"ledger-row", onclick:() => openIntent(intent) }, [node("span", { class:`ledger-state ${statusTone(intent.state)}`, text:statusCopy(intent.state) }), node("span", {}, [node("span", { class:"ledger-title", text:purchaseTitle(intent) }), node("span", { class:"ledger-meta", text:`${agentName(intent.agent_id)} · ${intent.merchant_domain} · ${when(intent.updated_at)}` }), reason ? node("span", { class:"ledger-reason", text:humanReason(reason) }) : null]), node("span", { class:"ledger-amount", text:money(intent.amount) })]);
}
function renderLedger() {
  let list = [...state.ledgerTransactions].sort((a,b) => b.updated_at - a.updated_at); const query = state.search.trim().toLowerCase();
  if (state.ledgerFilter === "waiting") list = list.filter((item) => waitingStates.has(item.state)); else if (state.ledgerFilter === "paid") list = list.filter((item) => paidStates.has(item.state)); else if (state.ledgerFilter === "stopped") list = list.filter((item) => stoppedStates.has(item.state));
  if (query) list = list.filter((item) => [purchaseTitle(item), item.merchant_domain, agentName(item.agent_id), item.provider_reference].filter(Boolean).some((value) => String(value).toLowerCase().includes(query)));
  replace($("#ledger-list"), list.length ? list.map(ledgerRow) : empty("No purchases match this filter."));
  $("#ledger-page-status").textContent = `Showing ${state.ledgerTransactions.length} of ${state.ledgerTotal} attempts`;
  $("#ledger-load-more").hidden = !state.ledgerCursor;
}
function renderAgents() {
  const agents = state.overview.agents; replace($("#agent-list"), agents.length ? agents.map(agentCard) : empty("No agents yet. Create one to issue a scoped capability."));
  const depositAgent = $("#deposit-agent"); const selected = depositAgent.value;
  replace(depositAgent, [node("option", { value:"", text:"No agent" }), ...agents.filter((agent) => !agent.revoked).map((agent) => node("option", { value:agent.id, text:agent.name }))]);
  depositAgent.value = selected;
}
function agentDescription(mode) {
  return ({ bounded_autonomous:"Spends quietly inside its standing limits", approval_required:"Asks before spending when your decision is needed", observe:"Reads only, never buys", disabled:"Spending is paused" })[mode] ?? "Works inside the boundaries you set";
}
function halveAgentAllowance(agent, policy) {
  const next = structuredClone(policy);
  next.max_rolling_24h.minor = Math.max(1, Math.floor(next.max_rolling_24h.minor / 2));
  next.max_per_transaction.minor = Math.max(1, Math.floor(next.max_per_transaction.minor / 2));
  confirmAction({
    title:`Halve ${agent.name}'s allowance?`,
    copy:"This immediately halves both the rolling 24-hour allowance and the most this agent may spend at once. Existing purchases are not changed.",
    facts:[["24-hour allowance",`${money(policy.max_rolling_24h)} → ${money(next.max_rolling_24h)}`],["Most at once",`${money(policy.max_per_transaction)} → ${money(next.max_per_transaction)}`]],
    label:"Halve allowance",
    action:()=>post("/api/policies/update",{agent_id:agent.id,policy:next},`${agent.name}'s allowance was halved.`),
  });
}
function restoreAgentSpending(agent, sessionExpired, sessionTtl) {
  if (agent.revoked || agent.expires_at <= Date.now()/1000) {
    const tokenFilename = `${agent.name.toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-|-$/g,"") || "agent"}-${Date.now()}.token`;
    confirmAction({title:`Let ${agent.name} spend again?`,copy:"Its old capability cannot be reused. Cixa will issue fresh approval-required access to a private local token file, with the same limits still in place.",label:"Let it spend",action:()=>post("/api/agents/rotate",{agent_id:agent.id,ttl_secs:86400,token_filename:tokenFilename},`${agent.name} can ask to spend again. Fresh access is approval-required.`)});
  } else if (sessionExpired) {
    confirmAction({title:`Let ${agent.name} spend again?`,copy:`This opens a new local spending session for ${durationMinutes(sessionTtl)}. Existing limits and approval rules still apply.`,label:"Let it spend",action:()=>post("/api/agents/arm-session",{agent_id:agent.id,ttl_secs:sessionTtl},`${agent.name} can spend inside its limits again.`)});
  } else setAgentMode(agent,"approval_required");
}
function agentCard(agent) {
  const policy = state.overview.policies[agent.policy_id]; const used = agent.budget?.usage?.rolling_24h_amount?.minor ?? 0; const limit = policy?.max_rolling_24h?.minor ?? 0; const capabilityExpired=agent.expires_at <= Date.now()/1000; const sessionExpired=agent.broker_session_expires_at <= Date.now()/1000; const spendMode=["approval_required","bounded_autonomous"].includes(agent.mode); const active = !agent.revoked && spendMode && !capabilityExpired && !sessionExpired; const status=agent.revoked?"Revoked":capabilityExpired?"Capability expired":agent.mode==="observe"?"Observe only":sessionExpired&&spendMode?"Session expired":active?"Active":"Paused"; const sessionTtl=policy?.card_session_ttl_secs ?? 600;
  const avatar=node("span",{class:"agent-avatar"},[svgIcon("icon-agents")]);
  const identity=node("button",{type:"button",class:"agent-identity",onclick:()=>openAgent(agent),attrs:{"aria-label":`Open ${agent.name} settings`}},[avatar,node("span",{class:"agent-copy"},[node("h2",{text:agent.name}),node("span",{class:"agent-subtitle",text:agentDescription(agent.mode)})])]);
  const allowance=button("Halve today's allowance","quiet-button agent-halve",()=>halveAgentAllowance(agent,policy));
  const toggle=button(active?"Pause spending":"Let it spend",active?"secondary-button agent-toggle":"quiet-button agent-toggle",()=>active?setAgentMode(agent,"disabled"):restoreAgentSpending(agent,sessionExpired,sessionTtl));
  return node("article", { class:"agent-card" }, [node("div", { class:"agent-head" }, [identity,node("span", { class:`state-badge ${active ? "success" : ""}`, text:status })]), node("div", { class:"agent-spend" }, [node("div", { class:"progress-row" }, [node("span", { text:"Used in the last 24 hours" }), node("strong", { text:`${money({minor:used,currency:policy?.primary_currency ?? "CAD"})} of ${money(policy?.max_rolling_24h)}` })]), node("progress", { class:"progress", value:limit ? Math.min(100,used/limit*100) : 0, max:100, attrs:{ "aria-label":`${agent.name} rolling-limit use` } })]), node("div", { class:"fact-list" }, [fact("Most it may spend at once", money(policy?.max_per_transaction)), fact("Purchases", String(agent.transaction_count ?? 0)), fact("Session ends", sessionExpired ? "Not armed" : when(agent.broker_session_expires_at))]), node("div", { class:"agent-actions" }, [allowance,toggle])]);
}
function fact(label, value) { return node("div", { class:"fact" }, [node("span", { text:label }), node("strong", { text:value })]); }
function renderTrust() {
  const provider = state.overview.provider; const card = provider.manual_card;
  const treasuryCurrency = provider.balance?.currency ?? "CAD"; $("#provider-form").elements.currency.value=treasuryCurrency;$("#provider-form").elements.currency.readOnly=true;$("#deposit-form").elements.currency.value=treasuryCurrency;$("#deposit-form").elements.currency.readOnly=true;
  replace($("#provider-summary"), [fact("Mode", title(provider.mode)), fact("Held as", card?.credential_reference_configured ? "Credential reference configured" : "No manual reference"), fact("Ends in", card?.last_four ?? "Not stored"), fact("Evidence", title(provider.balance_status))]);
  replace($("#system-summary"), [fact("Runs", "On this computer"), fact("Sends analytics", "Nothing"), fact("Records kept", "On this disk"), fact("Audit events", String(state.overview.audit_entry_count)), fact("Sanitized", state.overview.sanitized ? "Yes" : "No")]);
  if (!state.audit) { $("#audit-verification").textContent = "Audit history is temporarily unavailable. Other owner controls still work."; replace($("#audit-list"), empty("Refresh to try loading audit history again.")); }
  else {
    $("#audit-verification").textContent = state.audit.chain_valid ? `Audit chain verified · showing ${state.audit.entries.length} of ${state.audit.entries_total ?? state.audit.entries.length} events` : "Audit chain verification failed";
    replace($("#audit-list"), state.audit.entries.length ? state.audit.entries.map((entry) => node("article", { class:"audit-entry" }, [node("h3", { text:title(entry.action) }), node("p", { text:`${entry.actor} · ${when(entry.at)}${entry.intent_id ? ` · ${entry.intent_id}` : ""}` }), node("details", {}, [node("summary", { text:"Technical evidence" }), node("p", { text:`Sequence ${entry.sequence} · hash ${entry.hash} · previous ${entry.previous_hash}` }), node("pre", { text:JSON.stringify(entry.details, null, 2) })])])) : empty("No audit events yet."));
    $("#audit-load-more").hidden = !state.auditCursor;
  }
  const instructions = state.overview.receive_instructions; if (instructions) { const form = $("#receive-form"); form.elements.method.value = instructions.method; form.elements.address.value = instructions.address; form.elements.memo_template.value = instructions.memo_template; }
}

function navigate() {
  const requested = location.hash.slice(1); state.route = ["today","ledger","agents","trust"].includes(requested) ? requested : "today";
  $$('[data-view]').forEach((view) => { view.hidden = view.dataset.view !== state.route; });
  $$('[data-route]').forEach((link) => { if (link.dataset.route === state.route) link.setAttribute("aria-current","page"); else link.removeAttribute("aria-current"); });
  document.title = `${title(state.route)} · Cixa`; window.scrollTo({ top:0 });
}
function selectTrust(tab) { $$('[data-trust-tab]').forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.trustTab === tab))); $$('[data-trust-panel]').forEach((panel) => panel.hidden = panel.dataset.trustPanel !== tab); }
function closeDrawer() { const drawer=$("#detail-drawer"); drawer.classList.remove("open");drawer.setAttribute("aria-hidden","true"); document.body.classList.remove("drawer-open"); $(".app-shell").inert=false; $(".mobile-nav").inert=false; state.lastFocus?.focus(); }
function openDrawer(content) { state.lastFocus=document.activeElement; replace($("#drawer-content"),content); const drawer=$("#detail-drawer");drawer.classList.add("open");drawer.setAttribute("aria-hidden","false");document.body.classList.add("drawer-open");$(".app-shell").inert=true;$(".mobile-nav").inert=true;$(".drawer-close").focus(); }
function detailSection(heading, contents) { return node("section", { class:"drawer-section" }, [node("h3", { text:heading }), ...[].concat(contents)]); }
async function openIntent(summary) {
  let intent = summary; let receipt = null;
  try { intent = await api(`/api/intents/${encodeURIComponent(summary.id)}`); if (intent.receipt_hash) receipt = await api(`/api/receipts/${encodeURIComponent(intent.id)}`); }
  catch (error) { toast(`Could not load current details: ${error.message}`, true); }
  const facts=intent.checkout_facts ?? {}; const flags=[facts.recurring&&"Recurring",facts.trial_auto_renew&&"Auto-renewing trial",facts.stored_card&&"Stored card",facts.tip_minor&&"Tip",facts.preauthorization&&"Preauthorization",facts.installments&&"Installments"].filter(Boolean);
  const actions=[];
  if(intent.state==="approval_required") actions.push(button("Allow once","primary-button",()=>confirmIntentDecision(intent,true)),button("Decline","secondary-button",()=>confirmIntentDecision(intent,false)));
  if(waitingStates.has(intent.state)&&intent.state!=="approval_required") actions.push(button("Reconcile","primary-button",()=>{closeDrawer();openReconcile(intent);}));
  if(intent.state==="approved"&&state.overview.provider.mode==="manual_prepaid_card") actions.push(button("Begin owner handoff","primary-button",()=>beginManualHandoff(intent)));
  if(intent.state==="executing"&&state.overview.provider.mode==="manual_prepaid_card") actions.push(button("Complete handoff","primary-button",()=>confirmAction({title:"Did you submit exactly once?",copy:"Only continue after the owner-only browser submission is finished. Cixa will mark the outcome unknown until you check the provider.",label:"Complete handoff",action:()=>post("/api/handoff/complete",{intent_id:intent.id},"Handoff recorded. Check the provider outcome.")})));
  const itemRows=(intent.items??[]).map((item)=>fact(`${item.quantity} × ${item.label}`,money({minor:item.unit_price_minor,currency:intent.amount.currency})));
  const redirectRows=(facts.redirect_chain??[]).map((url,index)=>fact(`Redirect ${index+1}`,url));
  openDrawer([node("p",{class:"eyebrow",text:statusCopy(intent.state)}),node("h2",{id:"drawer-title",text:money(intent.amount)}),node("p",{class:"decision-title",text:purchaseTitle(intent)}),node("p",{class:"decision-meta",text:`${agentName(intent.agent_id)} · ${intent.merchant_domain}`}),detailSection("Why Cixa decided this",intent.decision?.reasons?.length?intent.decision.reasons.map((reason)=>node("p",{text:humanReason(reason)})):node("p",{text:"No policy exception was recorded."})),detailSection("Bound purchase",[fact("Requested",money(intent.requested_amount)),fact("Final total",money(intent.final_total)),...itemRows,fact("Fulfillment",intent.fulfillment_profile)]),detailSection("Checkout facts",[fact("Payment form",title(facts.payment_form)),fact("Scenario",title(facts.scenario)),fact("Risk flags",flags.join(", ")||"None"),...redirectRows]),receipt?detailSection("Sanitized receipt",[fact("Status",statusCopy(receipt.status)),fact("Issued",when(receipt.issued_at)),fact("Provider reference",receipt.provider_reference??"Not available"),fact("Personal details",receipt.personal_information_redacted?"Redacted":"Not redacted")]):null,detailSection("Evidence",[fact("Intent",intent.id),fact("Policy version",String(intent.policy_version)),fact("Created",when(intent.created_at)),fact("Updated",when(intent.updated_at)),fact("Provider reference",intent.provider_reference??"Not available"),fact("Receipt hash",intent.receipt_hash??"Not issued")]),actions.length?node("div",{class:"card-actions"},actions):null]);
}
function confirmIntentDecision(intent, approve) { closeDrawer(); confirmAction({title:approve?"Allow this one purchase?":"Decline this purchase?",copy:approve?"This approval applies once and does not permanently trust the merchant.":"The request will be cancelled and cannot restart.",facts:[["Amount",money(intent.amount)],["Merchant",intent.merchant_domain],["Agent",agentName(intent.agent_id)]],label:approve?"Allow once":"Decline",danger:!approve,action:()=>post(approve?"/api/approvals/approve":"/api/approvals/deny",{intent_id:intent.id},approve?"Purchase allowed once.":"Purchase declined.")}); }
function beginManualHandoff(intent) { closeDrawer(); const acknowledged=node("input",{type:"checkbox",required:true}); confirmAction({title:"Prepare the owner handoff?",copy:"Cixa will reserve the funds and mark this intent as executing before you open the merchant in an owner-only browser.",facts:[["Amount",money(intent.amount)],["Merchant",intent.merchant_domain]],custom:[node("label",{class:"checkbox-row"},[acknowledged," I will keep the agent suspended and submit at most once"])],label:"Prepare handoff",action:async()=>{const result=await post("/api/handoff/begin",{intent_id:intent.id},"Owner handoff is ready.");window.setTimeout(()=>showHandoff(result),0);}}); }
function showHandoff(result) { const intent=result.intent;const facts=intent.checkout_facts??{};openDrawer([node("p",{class:"eyebrow",text:"Owner-only handoff"}),node("h2",{id:"drawer-title",text:"Verify before paying"}),node("p",{class:"safety-note",text:result.instructions}),detailSection("Money",[fact("Requested",money(intent.requested_amount)),fact("Final total",money(intent.final_total))]),detailSection("Items",(intent.items??[]).map((item)=>fact(`${item.quantity} × ${item.label}`,money({minor:item.unit_price_minor,currency:intent.amount.currency})))),detailSection("Merchant and delivery",[fact("Merchant",intent.merchant_domain),fact("Fulfillment",intent.fulfillment_profile),...(facts.redirect_chain??[]).map((url,index)=>fact(`Redirect ${index+1}`,url))]),detailSection("Consent facts",[fact("Recurring",facts.recurring?"Yes":"No"),fact("Trial auto-renew",facts.trial_auto_renew?"Yes":"No"),fact("Stored card",facts.stored_card?"Yes":"No"),fact("Form trust",title(facts.payment_form))]),node("p",{class:"safety-note",text:"If any fact differs in the owner-only browser, do not submit. Never paste payment details into Cixa."}),button("I submitted exactly once","primary-button",()=>confirmAction({title:"Finish this handoff?",copy:"Cixa will record the outcome as unknown. You must check the provider before marking it paid or declined.",label:"Complete handoff",action:()=>post("/api/handoff/complete",{intent_id:intent.id},"Handoff recorded as unknown. Check the provider now.")}))]); }
function openAgent(agent) {
  const policy=state.overview.policies[agent.policy_id]; const sessionTtl=policy.card_session_ttl_secs;const canOperate=!agent.revoked&&agent.expires_at>Date.now()/1000;const modeSelect=node("select",{disabled:!canOperate},["observe","approval_required","bounded_autonomous","disabled"].map((mode)=>node("option",{value:mode,text:title(mode),selected:agent.mode===mode})));
  const merchantInput=node("input",{placeholder:"merchant.example.test",maxLength:253});
  openDrawer([
    node("p", { class:"eyebrow", text:agent.revoked ? "Revoked" : title(agent.mode) }),
    node("h2", { id:"drawer-title", text:agent.name }),
    node("p", { class:"decision-meta", text:`Capability expires ${when(agent.expires_at)}` }),
    detailSection("Authority", [
      node("label", { class:"stacked-form" }, [node("span", { text:"Mode" }), modeSelect]),
      canOperate?node("div", { class:"card-actions" }, [
        button("Save mode", "primary-button", () => setAgentMode(agent, modeSelect.value)),
        button(`Arm ${durationMinutes(sessionTtl)}`, "quiet-button", () => { closeDrawer(); confirmAction({ title:`Arm this agent for ${durationMinutes(sessionTtl)}?`, copy:"This opens a spending session inside the current policy and autonomy mode.", facts:[["Agent",agent.name],["Mode",title(agent.mode)]], label:"Arm session", action:() => post("/api/agents/arm-session", { agent_id:agent.id, ttl_secs:sessionTtl }, `Agent session armed for ${durationMinutes(sessionTtl)}.`) }); }),
      ]):node("div",{},[node("p",{class:"safety-note",text:"This capability cannot operate. Rotate it to invalidate the old token and issue fresh approval-required access."}),button("Rotate capability","quiet-button",()=>{closeDrawer();openRotateAgent(agent);})]),
    ]),
    detailSection("Limits", [
      fact("Per purchase", money(policy.max_per_transaction)), fact("Per session", money(policy.max_per_session)),
      fact("Rolling 24 hours", money(policy.max_rolling_24h)), fact("Lifetime", money(policy.max_lifetime)),
      button("Edit policy", "quiet-button", () => openPolicy(agent, policy)),
    ]),
    detailSection("Trusted merchants", [
      node("p", { text:agent.approved_merchants.length ? agent.approved_merchants.join(", ") : "No durable merchant approvals." }),
      node("div", { class:"card-actions" }, [merchantInput, button("Trust merchant", "quiet-button", () => { const merchant=merchantInput.value.trim(); closeDrawer(); confirmAction({ title:"Trust this merchant for future purchases?", copy:"This is broader than a one-time approval. The merchant will join this agent's durable allowlist.", facts:[["Agent",agent.name],["Merchant",merchant]], label:"Trust merchant", action:() => post("/api/merchants/approve", { agent_id:agent.id, merchant_domain:merchant }, "Merchant added to this agent's policy.") }); })]),
    ]),
    agent.revoked ? null : detailSection("Capability", button("Revoke capability", "secondary-button", () => { closeDrawer(); confirmAction({ title:"Revoke this capability?", copy:"This token stops working immediately. You can later rotate the agent to a fresh approval-required token.", label:"Revoke capability", danger:true, action:() => post("/api/agents/revoke", { agent_id:agent.id }, "Capability revoked.") }); })),
  ]);
}
async function applyAgentMode(agent,mode){closeDrawer();await post("/api/agents/mode",{agent_id:agent.id,mode},`${agent.name} is now ${title(mode).toLowerCase()}.`);}
function setAgentMode(agent,mode){const rank={disabled:0,observe:0,approval_required:1,bounded_autonomous:2};if((rank[mode]??0)>(rank[agent.mode]??0)){if($("#detail-drawer").classList.contains("open"))closeDrawer();confirmAction({title:`Give ${agent.name} more authority?`,copy:"This changes what the agent may do on future requests. Its existing policy limits still apply.",facts:[["Current mode",title(agent.mode)],["New mode",title(mode)]],label:"Change authority",action:()=>applyAgentMode(agent,mode)});}else return applyAgentMode(agent,mode);}
function openPolicy(agent,policy) {
  closeDrawer(); const specs=[["max_per_transaction","Most per purchase"],["max_per_session","Most per session"],["max_rolling_24h","Rolling 24-hour limit"],["max_lifetime","Lifetime limit"],["absolute_exposure_ceiling","Absolute exposure ceiling"],["max_treasury_size","Maximum treasury size"]];
  const fields=specs.map(([key,label])=>{const input=node("input",{type:"number",min:"0.01",step:"0.01",value:(policy[key].minor/100).toFixed(2),required:true,dataset:{policyKey:key}});return node("label",{},[`${label} (currently ${money(policy[key])})`,input]);});
  const booleans=[["require_approval_for_new_merchants","Ask before a new merchant"],["allow_recurring","Allow recurring charges"],["allow_trials","Allow trials"],["allow_stored_card","Allow stored cards"],["allow_tips","Allow tips"],["allow_preauthorization","Allow preauthorizations"],["allow_installments","Allow installments"]].map(([key,label])=>node("label",{class:"checkbox-row"},[node("input",{type:"checkbox",checked:policy[key],dataset:{policyBoolean:key}}),label]));
  confirmAction({title:`Edit ${agent.name}'s limits`,copy:"Raising a limit expands what this agent can spend. Cixa validates the complete policy before saving.",custom:[node("div",{class:"dialog-fields"},[...fields,...booleans])],label:"Save policy",action:()=>{const next=structuredClone(policy);$$('[data-policy-key]').forEach((input)=>{next[input.dataset.policyKey]={minor:Math.round(Number(input.value)*100),currency:policy.primary_currency};});$$('[data-policy-boolean]').forEach((input)=>{next[input.dataset.policyBoolean]=input.checked;});return post("/api/policies/update",{agent_id:agent.id,policy:next},"Policy saved.");}});
}
function openCreateAgent() {
  const policy=structuredClone(Object.values(state.overview.policies)[0]); if(!policy){toast("Create the initial policy from the CLI first.",true);return;} const name=node("input",{required:true,maxLength:80,placeholder:"Research runner"});const filename=node("input",{required:true,maxLength:64,placeholder:"research-runner.token"});const mode=node("select",{},["approval_required","bounded_autonomous","observe","disabled"].map((value)=>node("option",{value,text:title(value)})));
  confirmAction({title:"Create an agent",copy:"Cixa writes the scoped capability to a private local file. The token never appears in this page.",custom:[node("div",{class:"dialog-fields"},[node("label",{},["Name",name]),node("label",{},["Token filename",filename]),node("label",{},["Starting mode",mode])])],label:"Create agent",action:()=>post("/api/agents/create",{name:name.value.trim(),token_filename:filename.value.trim(),policy,mode:mode.value,ttl_secs:86400},"Agent created. Its capability was written to the private token directory.")});
}
function openRotateAgent(agent) { const filename=node("input",{required:true,maxLength:64,placeholder:`${agent.name.toLowerCase().replace(/[^a-z0-9]+/g,"-")}.token`});confirmAction({title:`Rotate ${agent.name}'s capability?`,copy:"The previous token stays invalid. Cixa writes one fresh token to a new private local file and resets the agent to approval-required mode.",custom:[node("div",{class:"dialog-fields"},[node("label",{},["New token filename",filename])])],label:"Rotate capability",action:()=>post("/api/agents/rotate",{agent_id:agent.id,ttl_secs:86400,token_filename:filename.value.trim()},"Capability rotated. Fresh access is approval-required.")});}
function openReconcile(intent) {
  const outcome=node("select",{},[node("option",{value:"settled",text:"It was paid"}),node("option",{value:"declined",text:"It was declined"})]);const reference=node("input",{required:true,maxLength:256,placeholder:"Reference from provider"});
  confirmAction({title:"What did the provider say?",copy:"Check the provider's own app or website. Leaving this unresolved is safer than guessing.",facts:[["Amount",money(intent.amount)],["Merchant",intent.merchant_domain],["Attempted",when(intent.updated_at)]],custom:[node("div",{class:"dialog-fields"},[node("label",{},["Outcome",outcome]),node("label",{},["Provider reference",reference])])],label:"Record outcome",action:()=>post("/api/reconcile",{intent_id:intent.id,outcome:outcome.value,provider_reference:reference.value.trim()},"Provider outcome recorded.")});
}
function confirmAction({title:heading,copy,facts=[],custom=[],label,danger=false,action}) {
  state.dialogAction=action; state.lastFocus=document.activeElement; $("#dialog-title").textContent=heading; const contents=[node("p",{text:copy})]; if(facts.length)contents.push(node("div",{class:"fact-list"},facts.map(([key,value])=>fact(key,value)))); contents.push(...custom);replace($("#dialog-body"),contents);const confirm=$("#dialog-confirm");confirm.textContent=label;confirm.className=danger?"secondary-button":"primary-button";$("#action-dialog").showModal();
}
async function exportAudit(){const value=await api("/api/export");const blob=new Blob([JSON.stringify(value,null,2)],{type:"application/json"});const url=URL.createObjectURL(blob);const link=node("a",{href:url,download:"cixa-sanitized-export.json"});link.click();window.setTimeout(()=>URL.revokeObjectURL(url),0);toast("Sanitized audit export prepared.");}

window.addEventListener("hashchange",navigate);
$$('[data-filter]').forEach((buttonItem)=>buttonItem.addEventListener("click",()=>{state.ledgerFilter=buttonItem.dataset.filter;$$('[data-filter]').forEach((item)=>item.setAttribute("aria-pressed",String(item===buttonItem)));renderLedger();}));
$("#ledger-search").addEventListener("input",(event)=>{state.search=event.target.value;renderLedger();});
$("#ledger-load-more").addEventListener("click",()=>loadLedger().catch((error)=>toast(error.message,true)));
$("#audit-load-more").addEventListener("click",()=>loadAudit().catch((error)=>toast(error.message,true)));
$$('[data-trust-tab]').forEach((buttonItem)=>buttonItem.addEventListener("click",()=>selectTrust(buttonItem.dataset.trustTab)));
$$('[data-close-drawer]').forEach((item)=>item.addEventListener("click",closeDrawer));
document.addEventListener("keydown",(event)=>{const drawer=$("#detail-drawer");if(!drawer.classList.contains("open"))return;if(event.key==="Escape")closeDrawer();if(event.key==="Tab"){const focusable=$$("#detail-drawer button:not([disabled]),#detail-drawer input:not([disabled]),#detail-drawer select:not([disabled]),#detail-drawer a[href]").filter((item)=>item.offsetParent!==null);if(!focusable.length)return;const first=focusable[0],last=focusable.at(-1);if(event.shiftKey&&document.activeElement===first){event.preventDefault();last.focus();}else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus();}}});
$("#action-dialog").addEventListener("close",()=>{state.dialogAction=null;state.lastFocus?.focus();});
$("#dialog-form").addEventListener("submit",async(event)=>{if(event.submitter?.value!=="confirm")return;event.preventDefault();if(!event.currentTarget.reportValidity())return;const action=state.dialogAction;try{await action?.();$("#action-dialog").close();}catch{}});
$("#emergency-stop-button").addEventListener("click",()=>confirmAction({title:"Stop all spending?",copy:"Every agent stops buying immediately. Requests already invalidated will not restart later.",label:"Stop all spending",danger:true,action:()=>post("/api/emergency-stop",{stopped:true},"Spending stopped.")}));
$("#resume-button").addEventListener("click",()=>confirmAction({title:"Let agents spend again?",copy:"Limits and standing rules are exactly as you left them. Cancelled requests stay cancelled.",label:"Start again",action:()=>post("/api/emergency-stop",{stopped:false},"Cixa is watching again.")}));
$("#refresh-button").addEventListener("click",()=>refresh().then(()=>toast("Up to date.")).catch((error)=>toast(error.message,true)));
$("#create-agent-button").addEventListener("click",openCreateAgent);
$("#export-button").addEventListener("click",()=>exportAudit().catch((error)=>toast(error.message,true)));
$("#provider-form").addEventListener("submit",(event)=>{event.preventDefault();const form=new FormData(event.currentTarget);confirmAction({title:"Save this provider reference?",copy:"Cixa stores the reference and masked last four, not card credentials. The balance is based on your confirmation.",label:"Save reference",action:()=>post("/api/provider/manual",{credential_reference:String(form.get("credential_reference")),provider_kind:String(form.get("provider_kind")),last_four:String(form.get("last_four")),balance:{minor:Math.round(Number(form.get("balance"))*100),currency:String(form.get("currency")).toUpperCase()},balance_status:String(form.get("balance_status")),balance_ttl_secs:Number(form.get("balance_ttl_minutes"))*60},"Provider reference saved.")});});
$("#receive-form").addEventListener("submit",(event)=>{event.preventDefault();const form=new FormData(event.currentTarget);return post("/api/receive",{method:String(form.get("method")),address:String(form.get("address")),memo_template:String(form.get("memo_template"))},"Receiving instructions saved.").catch(()=>{});});
$("#deposit-form").addEventListener("submit",(event)=>{event.preventDefault();const form=new FormData(event.currentTarget);const verified=form.get("verified")==="on";confirmAction({title:verified?"Confirm this money arrived?":"Record this as unverified?",copy:verified?"You are asserting that the provider's own record shows this money cleared. It may increase the linked agent's spending authority.":"Cixa will keep this arrival outside spending authority until an owner verifies it.",label:verified?"Record verified arrival":"Keep unverified",action:()=>post("/api/deposits/record",{amount:{minor:Math.round(Number(form.get("amount"))*100),currency:String(form.get("currency")).toUpperCase()},source:String(form.get("source")),verified,agent_id:String(form.get("agent_id"))||null,external_reference:String(form.get("external_reference"))},verified?"Verified arrival recorded.":"Unverified arrival recorded and kept outside spending authority.")});});
$("#theme-button").addEventListener("click",()=>{const dark=document.documentElement.dataset.theme!=="dark";document.documentElement.dataset.theme=dark?"dark":"light";localStorage.setItem("cixa-theme",dark?"dark":"light");$("#theme-button").setAttribute("aria-label",dark?"Use light theme":"Use dark theme");});
if(localStorage.getItem("cixa-theme")==="dark") { document.documentElement.dataset.theme="dark"; $("#theme-button").setAttribute("aria-label", "Use light theme"); }
navigate();
$("#unlock-form").addEventListener("submit",async(event)=>{event.preventDefault();const token=$("#unlock-token").value;$("#unlock-error").textContent="";try{const response=await fetch("/api/session",{method:"POST",credentials:"same-origin",headers:{"Content-Type":"application/json"},body:JSON.stringify({access_token:token})});if(!response.ok)throw new Error("That access token was not accepted.");$("#unlock-token").value="";readCsrf();$("#unlock-dialog").close();await refresh();}catch(error){$("#unlock-error").textContent=error.message;}});
if (csrf) refresh().catch((error)=>{if(error.message.includes("session required")){$("#unlock-dialog").showModal();$("#unlock-token").focus();return;}$("#connection-label").textContent="Cixa is offline";$("#watch-copy").textContent="The local broker did not answer.";toast(error.message,true);});
else { $("#unlock-dialog").showModal(); $("#unlock-token").focus(); }
window.setInterval(() => refresh().catch(() => {}), 15000);
document.addEventListener("visibilitychange", () => { if (!document.hidden) refresh().catch(() => {}); });
