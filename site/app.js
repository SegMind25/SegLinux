// SegLinux website: theme, navigation, screenshots and the list of ISO
// images. The list comes from releases.json, written when the site is
// deployed, and is refreshed from GitHub's API when the browser can reach it.
"use strict";

const REPO = "SegMind25/SegLinux";
const RELEASES_PAGE = `https://github.com/${REPO}/releases`;
let reduceMotion = document.documentElement.dataset.motion === "reduced";
window.addEventListener("seglinux-motion-change", () => {
  reduceMotion = document.documentElement.dataset.motion === "reduced";
  if (reduceMotion) {
    document.getElementById("orbit")?.style.removeProperty("--spin");
    document.querySelectorAll(".reveal-ready").forEach(finishReveal);
  }
});

// ---- Navigation --------------------------------------------------------

const menu = document.getElementById("menu-toggle");
const links = document.getElementById("nav-links");
menu.addEventListener("click", () => {
  const open = links.classList.toggle("open");
  menu.setAttribute("aria-expanded", String(open));
});
links.addEventListener("click", (event) => {
  if (event.target.closest("a")) {
    links.classList.remove("open");
    menu.setAttribute("aria-expanded", "false");
  }
});

function closeMenu() { links.classList.remove("open"); menu.setAttribute("aria-expanded", "false"); }
document.addEventListener("keydown", event => { if (event.key === "Escape" && links.classList.contains("open")) { closeMenu(); menu.focus(); } });
document.addEventListener("click", event => { if (!event.target.closest(".nav")) closeMenu(); });
window.matchMedia("(min-width: 761px)").addEventListener("change", event => { if (event.matches) closeMenu(); });

// The ruler on the right edge: a tick every 55 px, numbered every fifth,
// with a marker that follows the scroll position.
const ticks = document.getElementById("ruler-ticks");
const marker = document.getElementById("ruler-marker");
function drawRuler() {
  ticks.textContent = "";
  const height = ticks.clientHeight;
  for (let i = 0, y = 12; y < height - 4; i++, y += 55) {
    const tick = document.createElement("span");
    tick.style.top = `${y}px`;
    if (i % 5 === 0) {
      tick.className = "major";
      const number = document.createElement("b");
      number.textContent = i;
      number.style.top = `${y}px`;
      ticks.append(number);
    }
    ticks.append(tick);
  }
}

const orbit = document.getElementById("orbit");
const hero = document.getElementById("home");
// The orbit only turns while the hero is on screen: its endless tick
// animation repaints a large blurred SVG, which costs battery for nothing
// once the visitor has scrolled past it (or switched tabs).
let heroVisible = true;
new IntersectionObserver(([entry]) => {
  heroVisible = entry.isIntersecting;
  document.documentElement.classList.toggle("hero-hidden", !heroVisible);
}).observe(hero);
let ticking = false;
function onScroll() {
  if (ticking) return;
  ticking = true;
  requestAnimationFrame(() => {
    const max = document.documentElement.scrollHeight - window.innerHeight;
    const progress = max > 0 ? window.scrollY / max : 0;
    // A transform moves the marker without a layout on every scrolled frame.
    marker.style.transform = `translateY(${progress * Math.max(ticks.clientHeight - 24, 0)}px)`;
    if (!reduceMotion && heroVisible) orbit.style.setProperty("--spin", `${window.scrollY * 0.04}deg`);
    ticking = false;
  });
}
window.addEventListener("scroll", onScroll, { passive: true });
window.addEventListener("resize", () => {
  drawRuler();
  onScroll();
});
drawRuler();
onScroll();

// Highlight the section in view, in the top bar and the index.
const sectionLinks = document.querySelectorAll(".section-index a");
const navLinks = document.querySelectorAll(".nav-links a");
const downloadPage = document.body.dataset.page === "download";
// The section each top-bar link stands for: "#features" for in-page links,
// and the sections the Download and Docs pages continue on the home page.
const PAGE_SECTIONS = { "/download": "download", "/docs": "documentation" };
const linkSection = (a) => { const href = a.getAttribute("href"); return href.startsWith("#") ? href.slice(1) : PAGE_SECTIONS[href]; };
if (downloadPage) navLinks.forEach(a => { a.classList.toggle("active", a.getAttribute("href") === "/download"); if(a.getAttribute("href") === "/download") a.setAttribute("aria-current", "page"); });
const observer = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const id = entry.target.id;
      sectionLinks.forEach((a) => a.classList.toggle("current", a.dataset.section === id));
      if (!downloadPage) navLinks.forEach(a => { const active = linkSection(a) === id; a.classList.toggle("active", active); if (active) a.setAttribute("aria-current", "location"); else a.removeAttribute("aria-current"); });
    }
  },
  { rootMargin: "-45% 0px -50% 0px" },
);
document.querySelectorAll("main > section").forEach((section) => observer.observe(section));

// ---- Screenshots -------------------------------------------------------

const SHOTS = {
  "start-menu": ["The SegLinux desktop with the Start menu open", "The Start menu: search, applications, settings and power, from the floating dock."],
  desktop: ["The SegLinux desktop", "The desktop: wallpaper, desktop icons and the floating dock with the tray and clock."],
  setup: ["SegLinux Setup, choosing the disk", "Setup, the graphical installer, in the live session: choosing the disk."],
  "boot-menu": ["The boot menu of the SegLinux USB stick", "The boot menu of the DVD or USB stick."],
};
const shot = document.getElementById("shot");
const tabs = [...document.querySelectorAll(".tabs button")];
let shotRequest = 0;
const frame = shot.closest("figure"); frame.id = "screenshot-panel"; frame.setAttribute("role", "tabpanel");
tabs.forEach((tab, index) => {
  tab.id = `screenshot-tab-${index}`; tab.setAttribute("aria-controls", frame.id); tab.tabIndex = index === 0 ? 0 : -1;
  tab.addEventListener("click", () => {
    const name = tab.dataset.shot;
    if (tab.getAttribute("aria-selected") === "true" && shot.getAttribute("src") === `/assets/${name}.webp`) return;
    const request = ++shotRequest;
    // The tab answers at once; the picture fades over when it has loaded,
    // instead of the tab waiting for the download and the image jumping.
    tabs.forEach(t => { t.setAttribute("aria-selected", String(t === tab)); t.tabIndex = t === tab ? 0 : -1; });
    frame.setAttribute("aria-labelledby", tab.id);
    document.getElementById("shot-name").textContent = `seglinux — ${tab.textContent.toLowerCase()}`;
    frame.setAttribute("aria-busy", "true");
    const next = new Image();
    next.onload = () => {
      if (request !== shotRequest) return;
      shot.src = next.src; shot.alt = SHOTS[name][0];
      document.getElementById("shot-caption").textContent = SHOTS[name][1];
      frame.removeAttribute("aria-busy");
    };
    next.onerror = () => {
      if (request !== shotRequest) return;
      frame.removeAttribute("aria-busy");
      document.getElementById("shot-caption").textContent = "This screenshot could not load. Please try again.";
    };
    next.src = `/assets/${name}.webp`;
  });
  tab.addEventListener("keydown", event => {
    let next;
    if(event.key === "ArrowRight") next = (index + 1) % tabs.length;
    if(event.key === "ArrowLeft") next = (index + tabs.length - 1) % tabs.length;
    if(event.key === "Home") next = 0;
    if(event.key === "End") next = tabs.length - 1;
    if(next !== undefined) { event.preventDefault(); tabs[next].focus(); tabs[next].click(); }
  });
});
frame.setAttribute("aria-labelledby", tabs[0].id);

// ---- Copy buttons ------------------------------------------------------

document.querySelectorAll(".code .copy").forEach((button) => {
  let reset;
  button.addEventListener("click", async () => {
    const code = button.parentElement.querySelector("code");
    try {
      await navigator.clipboard.writeText(code.innerText);
      button.textContent = "Copied";
    } catch (e) {
      // No clipboard access (plain http, permissions): select the command
      // so Ctrl+C copies it.
      getSelection().selectAllChildren(code);
      button.textContent = "Selected";
    }
    // A second click restarts the delay instead of resetting the label early.
    clearTimeout(reset);
    reset = setTimeout(() => (button.textContent = "Copy"), 1600);
  });
});

// ---- ISO images --------------------------------------------------------

// A release holds one or more images: seglinux-lite-openrc-20261012-x86_64.iso
// (Lite) or seglinux-openrc-...iso (full), whole or in .partNN pieces, each
// with a .sha256 file. Builds from main are tagged like
// iso-20261012-lite-openrc-linux-zen-run14; version tags (v1.0) hold both
// editions. Images from before the rename are called wincraft-*.
const TAG = /^iso-\d{8}-(?:(?:full|lite)-)?[a-z0-9]+-(linux(?:-[a-z0-9]+)?)-run(\d+)$/;
const FILE = /^(seglinux|wincraft)(-lite)?-([a-z0-9]+)-(\d{4})(\d{2})(\d{2})-x86_64\.iso$/;

function parseRelease(release) {
  if (release.draft) return [];
  const assets = release.assets || [];
  const tag = TAG.exec(release.tag_name || "");
  const kernel = tag ? tag[1] : (/\(\w+, (linux[\w-]*)\)/.exec(release.body || "") || [])[1] || "";
  const groups = new Map();
  for (const asset of assets) {
    if (!/\.iso(\.part\d+)?$/.test(asset.name)) continue;
    const base = asset.name.replace(/\.part\d+$/, "");
    if (!groups.has(base)) groups.set(base, []);
    groups.get(base).push(asset);
  }
  const builds = [];
  for (const [isoName, parts] of groups) {
    const file = FILE.exec(isoName);
    if (!file) continue;
    parts.sort((a, b) => a.name.localeCompare(b.name));
    builds.push({
      label: tag ? `#${tag[2]}` : release.tag_name,
      page: release.html_url || `${RELEASES_PAGE}/tag/${release.tag_name}`,
      date: new Date(Date.UTC(+file[4], +file[5] - 1, +file[6])),
      published: new Date(release.published_at || release.created_at || 0),
      edition: file[2] ? "lite" : "full",
      init: file[3],
      kernel,
      size: parts.reduce((total, a) => total + a.size, 0),
      parts,
      checksum: assets.find((a) => a.name === `${isoName}.sha256`),
      isoName,
      previousName: file[1] === "wincraft",
    });
  }
  return builds;
}

function formatSize(bytes) {
  const gib = bytes / 1024 ** 3;
  return gib >= 1 ? `${(Math.ceil(gib * 10) / 10).toFixed(1)} GB` : `${Math.ceil(bytes / 1024 ** 2)} MB`;
}

const dateFormat = new Intl.DateTimeFormat("en", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === "class") node.className = value;
    else node.setAttribute(key, value);
  }
  node.append(...children);
  return node;
}

function fileLinks(build, primary) {
  const nodes = [];
  build.parts.forEach((part, i) => {
    const label = build.parts.length > 1 ? `Part ${i + 1} · ${formatSize(part.size)}` : `Download · ${formatSize(part.size)}`;
    nodes.push(el("a", { class: primary ? `btn ${i === 0 ? "btn-primary" : "btn-outline"} btn-small` : "", href: part.browser_download_url, download: part.name }, primary ? label : part.name.replace(/^.*\.(iso(\.part\d+)?)$/, "$1")));
  });
  if (build.checksum) {
    nodes.push(el("a", { class: primary ? "btn btn-outline btn-small" : "", href: build.checksum.browser_download_url, download: build.checksum.name }, "SHA-256"));
  }
  return nodes;
}

function renderEdition(card, build) {
  const signature = JSON.stringify(build);
  if (card.dataset.buildSignature === signature) return;
  const expanded = card.querySelector("details")?.open;
  card.dataset.buildSignature = signature;
  const field = (name) => card.querySelector(`[data-field="${name}"]`);
  if (!build) {
    field("meta").replaceChildren(el("li", {}, "No build published yet"));
    field("files").replaceChildren(el("p", { class: "muted" }, "This edition is not available yet."));
    return;
  }
  field("size").textContent = formatSize(build.size);
  field("meta").replaceChildren(
    el("li", {}, build.label.startsWith("#") ? `Build ${build.label.slice(1)}` : build.label),
    el("li", {}, dateFormat.format(build.date)),
    el("li", {}, [build.init, build.kernel].filter(Boolean).join(" · ")),
  );
  const files = build.parts.length > 1 ? [el("a", { class: "btn btn-primary", href: "/download-full.py", download: "download-full.py" }, "Get automatic Full ISO downloader") ] : fileLinks(build, true);
  if (build.parts.length > 1) {
    files.push(el("p", { class: "edition-note" }, "Run the downloaded helper with Python 3. It saves one ISO and checks its SHA-256 automatically. ", el("a", { href: "/docs/downloads.html" }, "How it works →")));
    files.push(el("details", {}, el("summary", {}, "Or download the parts manually"), ...fileLinks(build, true), el("p", { class: "edition-note" }, "Join in this order: ", el("code", {}, `cat ${build.isoName}.part* > ${build.isoName}`))));
  }
  field("files").replaceChildren(...files);
  if (expanded && card.querySelector("details")) card.querySelector("details").open = true;
}

function render(releases, source) {
  const builds = releases.flatMap(parseRelease).sort((a, b) => b.published - a.published || b.date - a.date);
  if (!builds.length) return false;
  for (const card of document.querySelectorAll(".edition")) {
    renderEdition(card, builds.find((b) => b.edition === card.dataset.edition));
  }
  // The sizes quoted at the top of the page follow the newest images too.
  const newest = (edition) => builds.find((b) => b.edition === edition);
  for (const node of document.querySelectorAll("[data-iso-size]")) {
    const build = newest(node.dataset.isoSize);
    if (build) node.textContent = formatSize(build.size);
  }
  const lite = newest("lite"), full = newest("full");
  for (const node of document.querySelectorAll("[data-iso-sizes]")) {
    if (lite && full) node.textContent = `Lite ${formatSize(lite.size)} · Full ${formatSize(full.size)}`;
  }
  const rows = builds.map((build) =>
    el(
      "tr",
      {},
      el("td", {}, dateFormat.format(build.date)),
      el("td", {}, build.edition === "lite" ? "Lite" : "Full", ...(build.previousName ? [el("span", { class: "tag", title: "Built before the project was renamed from Wincraft" }, "old name")] : [])),
      el("td", {}, build.label),
      el("td", {}, [build.init, build.kernel].filter(Boolean).join(" · ")),
      el("td", {}, formatSize(build.size)),
      el("td", {}, ...fileLinks(build, false)),
    ),
  );
  document.getElementById("builds").replaceChildren(...rows);
  const status = document.getElementById("builds-status");
  status.textContent = `${builds.length} image${builds.length === 1 ? "" : "s"} available · Select a file to download directly.`;
  return true;
}

async function loadReleases() {
  let shown = false;
  try {
    const response = await fetch("/releases.json", { cache: "no-cache" });
    if (response.ok) shown = render(await response.json(), "site");
  } catch (e) {
    // Opened from disk, or not deployed yet: try GitHub directly.
  }
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=100`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (response.ok) {
      const fresh = await response.json();
      if (Array.isArray(fresh) && render(fresh, "github")) shown = true;
    }
  } catch (e) {
    // Offline or rate limited: keep the list from the site.
  }
  if (!shown) {
    document.getElementById("builds-status").textContent = "Downloads are temporarily unavailable. Please try again shortly.";
    for (const card of document.querySelectorAll(".edition")) {
      card.querySelector('[data-field="files"]').replaceChildren(el("p", { class: "muted" }, "Download unavailable — please try again shortly."));
    }
  }
}
loadReleases();

// Gentle entrance motion, with all content visible when scripting is unavailable.
// Once an element has arrived, the reveal classes are removed again: left in
// place, their transition and transform would replace the element's own
// (the lift of the guide cards, the hover glow of the editions and features).
function finishReveal(node) {
  node.classList.remove("reveal-ready", "visible");
}
if (!reduceMotion) {
  const reveals = new IntersectionObserver(entries => entries.forEach(entry => {
    if (!entry.isIntersecting) return;
    const node = entry.target;
    reveals.unobserve(node);
    node.classList.add("visible");
    const done = (event) => { if (event.target === node && event.propertyName === "opacity") { node.removeEventListener("transitionend", done); finishReveal(node); } };
    node.addEventListener("transitionend", done);
    // In case the transition never runs (motion reduced meanwhile, hidden tab).
    setTimeout(() => finishReveal(node), 1000);
  }), { threshold: 0.08 });
  document.querySelectorAll(".feature,.edition,.doc-card,.gallery,.steps li").forEach(node => {
    node.classList.add("reveal-ready"); reveals.observe(node);
  });
}
const lightbox = document.createElement("dialog");
lightbox.className = "lightbox";
lightbox.innerHTML = '<button type="button" aria-label="Close screenshot">Close ×</button><img alt="">';
document.body.append(lightbox);
shot.tabIndex = 0; shot.setAttribute("role", "button"); shot.setAttribute("aria-label", "Enlarge screenshot");
function openShot() { const img = lightbox.querySelector("img"); img.src = shot.src; img.alt = shot.alt; lightbox.showModal(); }
shot.addEventListener("click", openShot);
shot.addEventListener("keydown", e => { if(e.key === "Enter" || e.key === " ") { e.preventDefault(); openShot(); } });
lightbox.querySelector("button").addEventListener("click", () => lightbox.close());
// Close on a click on the backdrop only, not on the dialog's own padding.
lightbox.addEventListener("click", e => {
  if (e.target !== lightbox) return;
  const r = lightbox.getBoundingClientRect();
  if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) lightbox.close();
});

lightbox.addEventListener("close", () => shot.focus({ preventScroll: true }));
