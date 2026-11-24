pragma Singleton
import QtQuick

QtObject {
    id: theme

    property SystemPalette sysPalette: SystemPalette { colorGroup: SystemPalette.Active }

    readonly property bool isDark: sysPalette.window.hslLightness < 0.5

    readonly property color windowBg: isDark ? "#0b1120" : sysPalette.window
    readonly property color surfaceBg: isDark ? "#111827" : sysPalette.base
    readonly property color surfaceAltBg: isDark ? "#1e293b" : sysPalette.alternateBase
    readonly property color border: isDark ? "#334155" : "#cbd5e1"

    readonly property color textPrimary: isDark ? "#f1f5f9" : sysPalette.windowText
    readonly property color textSecondary: isDark ? "#94a3b8" : "#475569"
    readonly property color accent: "#0284c7"

    readonly property color positive: isDark ? "#34d399" : "#059669"
    readonly property color negative: isDark ? "#f87171" : "#dc2626"
    readonly property color warning: isDark ? "#fbbf24" : "#d97706"

    readonly property color modePaperBg: "#0369a1"
    readonly property color modeTestnetBg: "#b45309"
    readonly property color modeLiveBg: "#b91c1c"
    readonly property color modeLiveStripe: "#f97316"

    readonly property string monoFont: "monospace"

    function signColor(signVal) {
        if (signVal > 0) return positive
        if (signVal < 0) return negative
        return textSecondary
    }

    function levelColor(level) {
        if (level === "critical" || level === "error") return negative
        if (level === "warn") return warning
        return accent
    }
}
