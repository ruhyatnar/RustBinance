import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: sysPage

    property var store
    property var client

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        PanelCard {
            Layout.fillWidth: true
            title: "UNIX DOMAIN SOCKET IPC CONNECTION"
            subtitle: sysPage.store ? sysPage.store.connectionState.toUpperCase() : "DISCONNECTED"

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                TextField {
                    id: sockInput
                    text: sysPage.store ? sysPage.store.socketPath : ""
                    placeholderText: "/run/binance-bot/bot.sock"
                    Layout.fillWidth: true
                }

                Button {
                    text: "Connect / Switch Socket"
                    onClicked: if (sysPage.client) sysPage.client.connectToDaemon(sockInput.text)
                }

                Button {
                    text: "Request Snapshot"
                    enabled: sysPage.store ? (sysPage.store.connectionState === "connected") : false
                    onClicked: if (sysPage.client) sysPage.client.requestSnapshot()
                }
            }

            Text {
                visible: sysPage.store ? (sysPage.store.lastError.length > 0) : false
                text: sysPage.store ? ("Last socket error: " + sysPage.store.lastError) : ""
                color: Theme.negative
                font.pixelSize: 12
                font.family: Theme.monoFont
            }
        }

        PanelCard {
            Layout.fillWidth: true
            title: "ENGINE & EXCHANGE TELEMETRY"

            GridLayout {
                columns: 2
                columnSpacing: 24
                rowSpacing: 8

                Text { text: "Engine Instance ID"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: (sysPage.store && sysPage.store.engineInstanceId.length > 0) ? sysPage.store.engineInstanceId : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }

                Text { text: "IPC Protocol Version"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: sysPage.store ? ("v" + sysPage.store.protocolVersion + " (seq " + sysPage.store.lastSeq + ")") : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }

                Text { text: "Build Mode / Engine Mode"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: sysPage.store ? (sysPage.store.buildMode.toUpperCase() + " / " + sysPage.store.engineMode) : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }

                Text { text: "Venues (Active / Spot / Futures)"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: sysPage.store ? (sysPage.store.activeVenue + " (" + sysPage.store.spotState + " / " + sysPage.store.futuresState + ")") : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }

                Text { text: "Daemon Uptime (s)"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: sysPage.store ? String(sysPage.store.uptimeS) : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }

                Text { text: "WebSocket Latency (ms)"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: sysPage.store ? (sysPage.store.wsLatencyMs + " ms") : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }

                Text { text: "Server Clock Drift (ms)"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: sysPage.store ? (sysPage.store.clockDriftMs + " ms") : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }

                Text { text: "API Weight Budget & 10s Orders"; color: Theme.textSecondary; font.pixelSize: 12 }
                Text { text: sysPage.store ? (sysPage.store.weightUsed + " / " + sysPage.store.weightLimit + " (orders_10s=" + sysPage.store.orders10s + ")") : "—"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }
            }
        }

        Item { Layout.fillHeight: true }
    }
}
