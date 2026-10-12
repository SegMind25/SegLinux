// SegLinux website: theme, navigation, screenshots and the list of ISO
// images. The list comes from releases.json, written when the site is
// deployed, and is refreshed from GitHub's API when the browser can reach it.
"use strict";

const REPO = "SegMind25/SegLinux";
const RELEASES_PAGE = `https://github.com/${REPO}/releases`;
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

// ---- Theme -------------------------------------------------------------

const root = document.documentElement;
document.getElementById("theme-toggle").addEventListener("click", () => {
  const next = root.dataset.theme === "light" ? "dark" : "light";
  root.dataset.theme = next;
  document.querySelector('meta[name="theme-color"]').content = next === "light" ? "#f3f2ee" : "#161719";
  try {
    localStorage.setItem("seglinux-theme", next);
  } catch (e) {
    // Private windows may refuse storage; the theme still switches.
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
let ticking = false;
function onScroll() {
  if (ticking) return;
  ticking = true;
  requestAnimationFrame(() => {
    const max = document.documentElement.scrollHeight - window.innerHeight;
    const progress = max > 0 ? window.scrollY / max : 0;
    marker.style.top = `${12 + progress * (ticks.clientHeight - 24)}px`;
    if (!reduceMotion) orbit.style.setProperty("--spin", `${window.scrollY * 0.04}deg`);
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
const navLinks = document.querySelectorAll(".nav-links a[href^='#']");
const observer = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const id = entry.target.id;
      sectionLinks.forEach((a) => a.classList.toggle("current", a.dataset.section === id));
      navLinks.forEach((a) => a.classList.toggle("active", a.getAttribute("href") === `#${id}`));
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
document.querySelectorAll(".tabs button").forEach((tab) => {
  tab.addEventListener("click", () => {
    const name = tab.dataset.shot;
    document.querySelectorAll(".tabs button").forEach((t) => t.setAttribute("aria-selected", String(t === tab)));
    shot.src = `assets/${name}.webp`;
    shot.alt = SHOTS[name][0];
    document.getElementById("shot-caption").textContent = SHOTS[name][1];
    document.getElementById("shot-name").textContent = `seglinux — ${tab.textContent.toLowerCase()}`;
  });
});

// ---- Copy buttons ------------------------------------------------------

document.querySelectorAll(".code .copy").forEach((button) => {
  button.addEventListener("click", async () => {
    const text = button.parentElement.querySelector("code").innerText;
    try {
      await navigator.clipboard.writeText(text);
      button.textContent = "Copied";
    } catch (e) {
      button.textContent = "Select";
    }
    setTimeout(() => (button.textContent = "Copy"), 1600);
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
    nodes.push(el("a", { class: primary ? `btn ${i === 0 ? "btn-primary" : "btn-outline"} btn-small` : "", href: part.browser_download_url }, primary ? label : part.name.replace(/^.*\.(iso(\.part\d+)?)$/, "$1")));
  });
  if (build.checksum) {
    nodes.push(el("a", { class: primary ? "btn btn-outline btn-small" : "", href: build.checksum.browser_download_url }, "SHA-256"));
  }
  return nodes;
}

function renderEdition(card, build) {
  const field = (name) => card.querySelector(`[data-field="${name}"]`);
  if (!build) {
    field("meta").replaceChildren(el("li", {}, "No build published yet"));
    return;
  }
  field("size").textContent = formatSize(build.size);
  field("meta").replaceChildren(
    el("li", {}, build.label.startsWith("#") ? `Build ${build.label.slice(1)}` : build.label),
    el("li", {}, dateFormat.format(build.date)),
    el("li", {}, [build.init, build.kernel].filter(Boolean).join(" · ")),
  );
  const files = fileLinks(build, true);
  if (build.parts.length > 1) {
    files.push(el("p", { class: "edition-note" }, "Join the parts after downloading: ", el("code", {}, `cat ${build.isoName}.part* > ${build.isoName}`)));
  }
  field("files").replaceChildren(...files);
}

function render(releases, source) {
  const builds = releases.flatMap(parseRelease).sort((a, b) => b.published - a.published || b.date - a.date);
  if (!builds.length) return false;
  for (const card of document.querySelectorAll(".edition")) {
    renderEdition(card, builds.find((b) => b.edition === card.dataset.edition));
  }
  const rows = builds.map((build) =>
    el(
      "tr",
      {},
      el("td", {}, dateFormat.format(build.date)),
      el("td", {}, build.edition === "lite" ? "Lite" : "Full", ...(build.previousName ? [el("span", { class: "tag", title: "Built before the project was renamed from Wincraft" }, "old name")] : [])),
      el("td", {}, el("a", { href: build.page }, build.label)),
      el("td", {}, [build.init, build.kernel].filter(Boolean).join(" · ")),
      el("td", {}, formatSize(build.size)),
      el("td", {}, ...fileLinks(build, false)),
    ),
  );
  document.getElementById("builds").replaceChildren(...rows);
  const status = document.getElementById("builds-status");
  status.replaceChildren(
    `${builds.length} image${builds.length === 1 ? "" : "s"}${source === "github" ? ", up to date from GitHub" : ""} · `,
    el("a", { href: RELEASES_PAGE }, "releases page"),
  );
  return true;
}

async function loadReleases() {
  let shown = false;
  try {
    const response = await fetch("releases.json", { cache: "no-cache" });
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
    document.getElementById("builds-status").replaceChildren(
      "The list of images could not be loaded. See the ",
      el("a", { href: RELEASES_PAGE }, "releases page"),
      ".",
    );
  }
}
loadReleases();
