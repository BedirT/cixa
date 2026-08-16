"use strict";

const csrf = document.cookie
  .split("; ")
  .find((part) => part.startsWith("csrf="))
  ?.split("=")[1] ?? "";

async function showStatus() {
  const response = await fetch("/api/status", { credentials: "same-origin" });
  document.querySelector("#status").textContent = JSON.stringify(await response.json(), null, 2);
}

async function setStop(stopped) {
  const response = await fetch("/api/emergency-stop", {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
    body: JSON.stringify({ stopped }),
  });
  document.querySelector("#status").textContent = JSON.stringify(await response.json(), null, 2);
}

document.querySelector("#stop").addEventListener("click", () => setStop(true));
document.querySelector("#resume").addEventListener("click", () => setStop(false));
void showStatus();
