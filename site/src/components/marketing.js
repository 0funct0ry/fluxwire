
const phases = [["DNS", 2, 14, "var(--cyan)"], ["Connect", 16, 22, "var(--green)"], ["TLS", 38, 41, "var(--purple)"], ["Waiting", 80, 54, "var(--blue)"], ["Download", 134, 8, "var(--orange)"]];
const total = 142, wf = document.getElementById("wf");
wf.innerHTML = phases.map(([n, s, d, c], i) =>
  `<span class="lbl">${n}</span><span class="track"><span class="bar" style="left:${s / total * 100}%;width:${Math.max(d / total * 100, 1)}%;background:${c};animation-delay:${200 + i * 260}ms"></span></span><span class="ms">${d} ms</span>`
).join("") + `<span class="lbl total">Total</span><span></span><span class="ms" style="color:var(--ink)">${total} ms</span>`;

const ua = navigator.userAgent, dl = document.getElementById("dl");
if (/Windows/.test(ua)) dl.innerHTML = "Download for Windows <small>x86_64</small>";
else if (/Linux/.test(ua) && !/Android/.test(ua)) dl.innerHTML = "Install on Linux <small>x86_64, arm64</small>";

document.querySelectorAll(".tabs button").forEach(b => b.addEventListener("click", () => {
  document.querySelectorAll(".tabs button").forEach(x => x.setAttribute("aria-selected", x === b));
  document.querySelectorAll(".panel").forEach(p => p.classList.toggle("on", p.id === "p-" + b.dataset.p));
}));
document.querySelectorAll("[data-copy]").forEach(b => b.addEventListener("click", () => {
  navigator.clipboard?.writeText(b.previousElementSibling.textContent);
  b.textContent = "Copied"; setTimeout(() => b.textContent = "Copy", 1400);
}));
document.getElementById("theme").addEventListener("click", () => {
  const r = document.documentElement;
  const dark = r.dataset.theme ? r.dataset.theme === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
  r.dataset.theme = dark ? "light" : "dark";
});
