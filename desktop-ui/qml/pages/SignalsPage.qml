import QtQuick
import QtQuick.Layouts
import BinanceBot

Item {
    id: sigPage

    property var store
    property var fmt

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        PanelCard {
            Layout.fillWidth: true
            Layout.fillHeight: true
            title: "SHADOW & STRATEGY SIGNALS"
            subtitle: (sigPage.store && sigPage.store.signalsModel)
                      ? (sigPage.store.signalsModel.count + " signals")
                      : ""

            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                Text { text: "Venue"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 64 }
                Text { text: "Symbol"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 96 }
                Text { text: "Strategy"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.fillWidth: true }
                Text { text: "Direction"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 96 }
                Text { text: "Entry"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 110; horizontalAlignment: Text.AlignRight }
                Text { text: "Stop"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 110; horizontalAlignment: Text.AlignRight }
            }

            ListView {
                id: listView
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: sigPage.store ? sigPage.store.signalsModel : null

                delegate: RowLayout {
                    width: listView.width
                    height: 30
                    spacing: 8

                    Text { text: model.venue.toUpperCase(); color: Theme.accent; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 64 }
                    Text { text: model.symbol; color: Theme.textPrimary; font.pixelSize: 12; font.bold: true; Layout.preferredWidth: 96 }
                    Text { text: model.strategy; color: Theme.textSecondary; font.pixelSize: 12; Layout.fillWidth: true }
                    Text {
                        readonly property bool isShort: model.direction.toUpperCase() === "SHORT"
                        text: (isShort ? "▼ " : "▲ ") + model.direction.toUpperCase()
                        color: isShort ? Theme.negative : Theme.positive
                        font.pixelSize: 12
                        font.bold: true
                        Layout.preferredWidth: 96
                    }
                    Text {
                        text: sigPage.fmt ? sigPage.fmt.formatPrice(model.entry) : model.entry
                        color: Theme.textPrimary
                        font.pixelSize: 12
                        font.family: Theme.monoFont
                        Layout.preferredWidth: 110
                        horizontalAlignment: Text.AlignRight
                    }
                    Text {
                        text: sigPage.fmt ? sigPage.fmt.formatPrice(model.stop) : model.stop
                        color: Theme.negative
                        font.pixelSize: 12
                        font.family: Theme.monoFont
                        Layout.preferredWidth: 110
                        horizontalAlignment: Text.AlignRight
                    }
                }
            }

            Text {
                visible: !sigPage.store || !sigPage.store.signalsModel || sigPage.store.signalsModel.count === 0
                text: "No shadow signals recorded."
                color: Theme.textSecondary
                font.pixelSize: 12
            }
        }

        EventList {
            Layout.fillWidth: true
            eventsModel: sigPage.store ? sigPage.store.eventsModel : null
            searchQuery: "signal"
            preferredListHeight: 180
        }
    }
}
