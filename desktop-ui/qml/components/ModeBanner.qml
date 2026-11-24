import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Rectangle {
    id: banner

    property var store
    property var fmt
    property bool commandBusy: false

    signal panicConfirmed()
    signal toggleControlsClicked()

    readonly property string modeLower: store ? store.buildMode.toLowerCase() : "paper"
    readonly property bool isLive: modeLower === "live"
    readonly property bool isTestnet: modeLower === "testnet"

    implicitHeight: 48
    color: isLive ? Theme.modeLiveBg : (isTestnet ? Theme.modeTestnetBg : Theme.modePaperBg)

    Canvas {
        id: liveStripes
        anchors.fill: parent
        visible: banner.isLive
        onPaint: {
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            ctx.strokeStyle = "rgba(249, 115, 22, 0.35)"
            ctx.lineWidth = 8
            for (var x = -height; x < width + height; x += 22) {
                ctx.beginPath()
                ctx.moveTo(x, height)
                ctx.lineTo(x + height, 0)
                ctx.stroke()
            }
        }
        onWidthChanged: if (visible) requestPaint()
        onHeightChanged: if (visible) requestPaint()
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        spacing: 12

        // Mode + Venue Badge (icon + text, never color alone)
        Rectangle {
            radius: 5
            color: "rgba(15, 23, 42, 0.55)"
            border.color: "rgba(255, 255, 255, 0.35)"
            implicitWidth: modeLabel.implicitWidth + 16
            implicitHeight: 28

            Text {
                id: modeLabel
                anchors.centerIn: parent
                text: (banner.isLive ? "⬢ LIVE" : (banner.isTestnet ? "▲ TESTNET" : "◈ PAPER"))
                      + " · " + (banner.store ? banner.store.activeVenue.toUpperCase() : "SPOT")
                color: "#ffffff"
                font.pixelSize: 12
                font.bold: true
            }
        }

        // Engine Mode Pill
        Rectangle {
            radius: 5
            color: "rgba(15, 23, 42, 0.65)"
            implicitWidth: engineLabel.implicitWidth + 16
            implicitHeight: 28

            Text {
                id: engineLabel
                anchors.centerIn: parent
                readonly property string em: banner.store ? banner.store.engineMode : "Starting"
                readonly property string glyph: (em === "Ready") ? "● " : ((em === "Killed" || em === "Halted") ? "✖ " : "❚❚ ")
                text: glyph + em.toUpperCase()
                color: "#ffffff"
                font.pixelSize: 12
                font.bold: true
            }
        }

        // Key Equity / Day PnL / Drawdown / Latency / Weight summary
        Text {
            Layout.fillWidth: true
            elide: Text.ElideRight
            color: "#ffffff"
            font.pixelSize: 12
            font.family: Theme.monoFont
            text: {
                if (!banner.store || !banner.fmt || !banner.store.hasSnapshot) {
                    var cs = banner.store ? banner.store.connectionState : "disconnected"
                    return "IPC: " + cs
                }
                var eq = banner.fmt.formatDecimal(banner.store.equity, 2)
                var dayGlyph = banner.fmt.directionGlyph(banner.store.dayPnlPct)
                var day = banner.fmt.formatDecimal(banner.store.dayPnlPct, 2)
                var dd = banner.fmt.formatDecimal(banner.store.drawdownPct, 2)
                return "Eq " + eq
                     + "   Day " + dayGlyph + " " + day + "%"
                     + "   DD " + dd + "%"
                     + "   WS " + banner.store.wsLatencyMs + "ms"
                     + "   W " + banner.store.weightUsed + "/" + banner.store.weightLimit
                     + "   🔔" + banner.store.unackedAlertCount
            }
        }

        Button {
            text: (banner.store && banner.store.controlsUnlocked) ? "🔓 Lock Controls" : "🔒 Unlock Controls"
            enabled: banner.store ? (banner.store.connectionState === "connected") : false
            onClicked: banner.toggleControlsClicked()
        }

        HoldToConfirmButton {
            visible: banner.store ? banner.store.canMutate : false
            enabledControl: banner.store ? banner.store.canMutate : false
            busy: banner.commandBusy
            label: "■ PANIC (Hold 1.5s)"
            onConfirmed: banner.panicConfirmed()
        }
    }
}
