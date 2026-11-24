import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: settingsPage

    property var store
    property var commands

    ScrollView {
        anchors.fill: parent
        contentWidth: availableWidth

        ColumnLayout {
            width: parent.width
            spacing: 12

            PanelCard {
                Layout.fillWidth: true
                title: "RUNTIME RISK PARAMETERS (TIGHTEN-ONLY · HARD RULE #8)"
                subtitle: (settingsPage.store && settingsPage.store.canMutate)
                          ? "Unlocked"
                          : "Read-only (unlock controls to tighten)"

                Text {
                    color: Theme.textSecondary
                    font.pixelSize: 12
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                    text: "Runtime updates are validated by the daemon and may ONLY tighten risk limits. Any attempt to loosen a limit is rejected."
                }

                GridLayout {
                    Layout.fillWidth: true
                    columns: 4
                    columnSpacing: 10
                    rowSpacing: 8

                    // 1. Portfolio Heat Max %
                    Text { text: "risk.portfolio_heat_max_pct"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    Text { text: settingsPage.store ? ("Current: " + settingsPage.store.heatMaxPct + "%") : "—"; color: Theme.textSecondary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    TextField {
                        id: heatMaxInput
                        text: settingsPage.store ? settingsPage.store.heatMaxPct : ""
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        Layout.fillWidth: true
                    }
                    Button {
                        text: "Tighten"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        onClicked: if (settingsPage.commands) settingsPage.commands.setParam("risk.portfolio_heat_max_pct", heatMaxInput.text)
                    }

                    // 2. Max Open Positions
                    Text { text: "risk.max_open_positions"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    Text { text: settingsPage.store ? ("Current: " + settingsPage.store.maxPositions) : "—"; color: Theme.textSecondary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    TextField {
                        id: maxPosInput
                        text: (settingsPage.store && settingsPage.store.hasSnapshot) ? String(settingsPage.store.maxPositions) : ""
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        Layout.fillWidth: true
                    }
                    Button {
                        text: "Tighten"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        onClicked: if (settingsPage.commands) settingsPage.commands.setParam("risk.max_open_positions", maxPosInput.text)
                    }

                    // 3. Daily Loss Halt %
                    Text { text: "risk.daily_loss_halt_pct"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    Text { text: settingsPage.store ? ("Current: " + settingsPage.store.dailyHaltPct + "%") : "—"; color: Theme.textSecondary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    TextField {
                        id: dailyHaltInput
                        text: settingsPage.store ? settingsPage.store.dailyHaltPct : ""
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        Layout.fillWidth: true
                    }
                    Button {
                        text: "Tighten"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        onClicked: if (settingsPage.commands) settingsPage.commands.setParam("risk.daily_loss_halt_pct", dailyHaltInput.text)
                    }

                    // 4. Drawdown Kill %
                    Text { text: "risk.drawdown_kill_pct"; color: Theme.textPrimary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    Text { text: settingsPage.store ? ("Current: " + settingsPage.store.killPct + "%") : "—"; color: Theme.textSecondary; font.family: Theme.monoFont; font.pixelSize: 12 }
                    TextField {
                        id: killInput
                        text: settingsPage.store ? settingsPage.store.killPct : ""
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        Layout.fillWidth: true
                    }
                    Button {
                        text: "Tighten"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        onClicked: if (settingsPage.commands) settingsPage.commands.setParam("risk.drawdown_kill_pct", killInput.text)
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    TextField {
                        id: customKeyInput
                        placeholderText: "Other whitelisted key (e.g. risk.risk_per_trade_pct)"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        Layout.fillWidth: true
                    }
                    TextField {
                        id: customValInput
                        placeholderText: "New tighter value"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        Layout.preferredWidth: 160
                    }
                    Button {
                        text: "Apply Tighten"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        onClicked: if (settingsPage.commands) settingsPage.commands.setParam(customKeyInput.text, customValInput.text)
                    }
                }
            }

            PanelCard {
                Layout.fillWidth: true
                title: "DISABLE STRATEGY AT RUNTIME"

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    ComboBox {
                        id: strategySelector
                        model: ["TrendPullback", "Breakout"]
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                    }
                    Button {
                        text: "Disable Selected Strategy"
                        enabled: settingsPage.store ? settingsPage.store.canMutate : false
                        onClicked: if (settingsPage.commands) settingsPage.commands.disableStrategy(strategySelector.currentText)
                    }
                    Item { Layout.fillWidth: true }
                }
            }
        }
    }
}
