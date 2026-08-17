"use strict";

const csrf = document.cookie
  .split("; ")
  .find((part) => part.startsWith("csrf="))
  ?.split("=")[1] ?? "";

async function readJson(response) {
  const value = await response.json();
  if (!response.ok) throw new Error(value.error ?? `request failed: ${response.status}`);
  return value;
}

async function showOverview() {
  const response = await fetch("/api/overview", { credentials: "same-origin" });
  const value = await readJson(response);
  document.querySelector("#status").textContent = JSON.stringify(value, null, 2);
  document.querySelector("#verification").textContent =
    `Balance evidence: ${value.provider.balance_status}. Provider mode: ${value.provider.mode}.`;
  const unsafe = document.querySelector("#unsafe");
  unsafe.hidden = value.unsafe_modes.length === 0;
  unsafe.textContent = value.unsafe_modes.length
    ? `EXPERIMENTAL OR UNSAFE MODE: ${value.unsafe_modes.join("; ")}`
    : "";
}

async function setStop(stopped) {
  const response = await fetch("/api/emergency-stop", {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
    body: JSON.stringify({ stopped }),
  });
  document.querySelector("#result").textContent = JSON.stringify(await readJson(response), null, 2);
  await showOverview();
}

async function submitOwnerOperation(form) {
  const endpoint = form.dataset.endpoint;
  const body = JSON.parse(form.querySelector("textarea").value);
  const response = await fetch(endpoint, {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
    body: JSON.stringify(body),
  });
  document.querySelector("#result").textContent = JSON.stringify(await readJson(response), null, 2);
  await showOverview();
}

async function loadAudit() {
  const response = await fetch("/api/audit", { credentials: "same-origin" });
  document.querySelector("#audit-output").textContent =
    JSON.stringify(await readJson(response), null, 2);
}

async function exportSanitized() {
  const response = await fetch("/api/export", { credentials: "same-origin" });
  const value = await readJson(response);
  const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = "cixa-sanitized-export.json";
  link.click();
  URL.revokeObjectURL(link.href);
}

document.querySelector("#stop").addEventListener("click", () => setStop(true));
document.querySelector("#resume").addEventListener("click", () => setStop(false));
document.querySelectorAll(".refresh").forEach((button) =>
  button.addEventListener("click", () => showOverview().catch(showError))
);
document.querySelector("#audit").addEventListener("click", () => loadAudit().catch(showError));
document.querySelector("#export").addEventListener("click", () => exportSanitized().catch(showError));
document.querySelectorAll("form[data-endpoint]").forEach((form) => {
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    submitOwnerOperation(form).catch(showError);
  });
});

function showError(error) {
  document.querySelector("#result").textContent = `Rejected: ${error.message}`;
}

void showOverview().catch(showError);
