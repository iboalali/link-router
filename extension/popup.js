const statusEl = document.getElementById("status");
const profileSelect = document.getElementById("profile-select");
const pauseToggle = document.getElementById("pause-toggle");
const openManagerBtn = document.getElementById("open-manager");
const refreshBtn = document.getElementById("refresh-btn");

function render(state) {
  if (!state) return;
  if (state.connected) {
    const count = Array.isArray(state.rules) ? state.rules.length : 0;
    statusEl.textContent = `Connected (${count} active rule${count === 1 ? "" : "s"})`;
    statusEl.className = "status ok";
  } else {
    statusEl.textContent = `Disconnected: ${state.lastError || "run link-router install --extension-id"}`;
    statusEl.className = "status err";
  }

  profileSelect.innerHTML = "";
  const unsetOpt = document.createElement("option");
  unsetOpt.value = "";
  unsetOpt.textContent = "Not set";
  profileSelect.appendChild(unsetOpt);

  const profiles = (state.profiles || []).filter((p) => p.kind === "chrome_profile" && p.profile_dir);
  for (const p of profiles) {
    const opt = document.createElement("option");
    opt.value = p.profile_dir;
    opt.textContent = p.email ? `${p.name} (${p.email})` : p.name;
    profileSelect.appendChild(opt);
  }
  profileSelect.value = state.source_profile || "";
  pauseToggle.checked = Boolean(state.paused);
}

function load() {
  chrome.runtime.sendMessage({ type: "popup_get_state" }, render);
}

profileSelect.addEventListener("change", () => {
  chrome.runtime.sendMessage(
    { type: "popup_set_profile", profile_dir: profileSelect.value || null },
    render
  );
});

pauseToggle.addEventListener("change", () => {
  chrome.runtime.sendMessage(
    { type: "popup_set_paused", paused: pauseToggle.checked },
    render
  );
});

openManagerBtn.addEventListener("click", () => {
  chrome.runtime.sendMessage({ type: "popup_open_manager" });
});

refreshBtn.addEventListener("click", load);

load();
