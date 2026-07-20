const PROCESS_GROUPS = {
    "chrome.exe": "Google Chrome",
    "msedge.exe": "Microsoft Edge",
    "firefox.exe": "Mozilla Firefox",
    "brave.exe": "Brave Browser",
    "opera.exe": "Opera Browser",
    "discord.exe": "Discord",
    "spotify.exe": "Spotify",
    "teams.exe": "Microsoft Teams",
    "slack.exe": "Slack",
    "zoom.exe": "Zoom",
    "steam.exe": "Steam",
    "devenv.exe": "Visual Studio",
    "code.exe": "VS Code",
    "tauri-app.exe": "Internet Speed Meter",
    "antigravity.exe": "Antigravity",
    "windowsterminal.exe": "Windows Terminal",
    "explorer.exe": "Windows Explorer",
    "system": "System",
    "idle": "Idle"
};

export function getDisplayName(processName) {
    if (!processName) return "Unknown";
    const lower = processName.toLowerCase();
    if (PROCESS_GROUPS[lower]) {
        return PROCESS_GROUPS[lower];
    }
    // Clean fallback name: strip .exe and capitalize
    let cleanName = processName;
    if (cleanName.toLowerCase().endsWith(".exe")) {
        cleanName = cleanName.substring(0, cleanName.length - 4);
    }
    return cleanName
        .split(/[_-]/)
        .map(word => word.charAt(0).toUpperCase() + word.slice(1))
        .join(" ");
}

export function groupProcesses(processList) {
    if (!processList || !Array.isArray(processList)) return [];

    const grouped = {};

    processList.forEach(item => {
        const displayName = getDisplayName(item.process_name);
        if (!grouped[displayName]) {
            grouped[displayName] = {
                process_name: displayName, // Render display name as process_name in UI
                bytes_downloaded: 0,
                bytes_uploaded: 0,
                screen_time_seconds: 0
            };
        }
        grouped[displayName].bytes_downloaded += item.bytes_downloaded || 0;
        grouped[displayName].bytes_uploaded += item.bytes_uploaded || 0;
        grouped[displayName].screen_time_seconds += item.screen_time_seconds || 0;
    });

    return Object.values(grouped).sort((a, b) => {
        const totalA = a.bytes_downloaded + a.bytes_uploaded;
        const totalB = b.bytes_downloaded + b.bytes_uploaded;
        return totalB - totalA;
    });
}
