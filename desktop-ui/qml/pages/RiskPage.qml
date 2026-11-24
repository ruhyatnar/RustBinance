import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import BinanceBot

Item {
    id: riskPage

    property var store
    property var commands
    property var fmt

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            RiskGauge {
                Layout.fillWidth: true
                store: riskPage.store
                fmt: riskPage.fmt
            }

            PanelCard {
                Layout.fillWidth: true
                title: "CIRCUIT BREAKERS & RECOVERY"
                subtitle: riskPage.store ? ("Engine: " + riskPage.store.engineMode) : ""

                Text {
                    color: Theme.textSecondary
                    font.pixelSize: 12
                    text: riskPage.store && riskPage.fmt
                          ? ("Daily Halt Threshold: " + riskPage.fmt.formatDecimal(riskPage.store.dailyHaltPct, 2) + "%\n"
                             + "Kill Drawdown Threshold: " + riskPage.fmt.formatDecimal(riskPage.store.killPct, 2) + "%\n"
                             + "Week PnL: " + riskPage.fmt.directionGlyph(riskPage.store.weekPnlPct) + " "
                             + riskPage.fmt.formatDecimal(riskPage.store.weekPnlPct, 2) + "%")
                          : "—"
                }

                HoldToConfirmButton {
                    visible: riskPage.store
                             ? (riskPage.store.canMutate && riskPage.store.engineMode === "Killed")
                             : false
                    enabledControl: riskPage.store ? riskPage.store.canMutate : false
                    busy: riskPage.commands ? riskPage.commands.busy : false
                    label: "↺ RESET KILL → PAUSED (Hold 1.5s)"
                    onConfirmed: if (riskPage.commands) riskPage.commands.executeConfirmedAction("reset_kill")
                }
            }
        }

        PanelCard {
            Layout.fillWidth: true
            Layout.fillHeight: true
            title: "OPERATOR ALERTS"
            subtitle: riskPage.store ? (riskPage.store.unackedAlertCount + " unacknowledged") : ""

            ListView {
                id: alertList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: riskPage.store ? riskPage.store.alertsModel : null

                delegate: RowLayout {
                    width: alertList.width
                    height: 34
                    spacing: 8

                    Text {
                        text: "[" + model.level.toUpperCase() + "]"
                        color: Theme.levelColor(model.level)
                        font.pixelSize: 11
                        font.bold: true
                        font.family: Theme.monoFont
                        Layout.preferredWidth: 78
                    }
                    Text {
                        visible: model.venue.length > 0
                        text: model.venue.toUpperCase()
                        color: Theme.accent
                        font.pixelSize: 11
                        font.bold: true
                        Layout.preferredWidth: 54
                    }
                    Text {
                        text: model.msg
                        color: model.acked ? Theme.textSecondary : Theme.textPrimary
                        font.pixelSize: 12
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                    }
                    Button {
                        visible: !model.acked && (riskPage.store ? riskPage.store.canMutate : false)
                        text: "Ack"
                        onClicked: if (riskPage.commands) riskPage.commands.ackAlert(model.alertId)
                    }
                }
            }

            Text {
                visible: !riskPage.store || !riskPage.store.alertsModel || riskPage.store.alertsModel.count === 0
                text: "No active alerts."
                color: Theme.textSecondary
                font.pixelSize: 12
            }
        }
    }
}
