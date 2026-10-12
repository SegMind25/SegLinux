// Wincraft default desktop layout (Plasma 6 scripting API).
//
// Plasma runs this once, the first time a user logs in, because
// /etc/xdg/kdeglobals selects this look-and-feel package. It builds a
// Windows 10 style taskbar:
//
//   [Start] [pinned + running programs ............] [tray] [clock] [|]
//
// The last item is "Peek at Desktop", the Show Desktop corner of Windows.

var panel = new Panel;
panel.location = "bottom";
panel.hiding = "none";
// Windows' taskbar is flush with the screen edge.
panel.floating = false;
// Same height formula as Plasma's own default panel: tall enough for an
// icons-only task manager, rounded to an even number.
panel.height = 2 * Math.floor(gridUnit * 2.5 / 2);

var start = panel.addWidget("org.kde.plasma.kickoff");
start.currentConfigGroup = ["Shortcuts"];
start.writeConfig("global", "Alt+F1");
start.currentConfigGroup = ["General"];
start.writeConfig("icon", "start-here-kde-symbolic");

var tasks = panel.addWidget("org.kde.plasma.icontasks");
tasks.currentConfigGroup = ["General"];
// Pinned like Explorer, Edge and the Store on a fresh Windows install.
tasks.writeConfig("launchers", [
    "preferred://filemanager",
    "preferred://browser",
    "applications:org.kde.discover.desktop",
    "applications:org.kde.konsole.desktop"
]);

panel.addWidget("org.kde.plasma.marginsseparator");
panel.addWidget("org.kde.plasma.systemtray");

var clock = panel.addWidget("org.kde.plasma.digitalclock");
clock.currentConfigGroup = ["Appearance"];
clock.writeConfig("showDate", "true");
clock.writeConfig("dateFormat", "shortDate");

panel.addWidget("org.kde.plasma.showdesktop");

// Desktop icons (Folder View) on the default wallpaper, like Windows.
var desktopsArray = desktopsForActivity(currentActivity());
for (var j = 0; j < desktopsArray.length; j++) {
    desktopsArray[j].wallpaperPlugin = "org.kde.image";
}
