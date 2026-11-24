import QtQuick
import QtQuick.Layouts
import BinanceBot

PanelCard {
    id: ladder
    title: "TOP-OF-BOOK WATCHLIST"

    property var ticksModel
    property var fmt
    signal symbolSelected(string symbol)

    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Text { text: "Venue"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 58 }
        Text { text: "Symbol"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.fillWidth: true }
        Text { text: "Bid"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 96; horizontalAlignment: Text.AlignRight }
        Text { text: "Ask"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 96; horizontalAlignment: Text.AlignRight }
        Text { text: "Last"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 96; horizontalAlignment: Text.AlignRight }
        Text { text: "Spread (bps)"; color: Theme.textSecondary; font.pixelSize: 11; font.bold: true; Layout.preferredWidth: 88; horizontalAlignment: Text.AlignRight }
    }

    ListView {
        id: listView
        Layout.fillWidth: true
        Layout.preferredHeight: Math.max(120, contentHeight)
        clip: true
        model: ladder.ticksModel

        delegate: Rectangle {
            width: listView.width
            height: 28
            color: (index % 2 === 0) ? "transparent" : Theme.surfaceAltBg
            radius: 4

            MouseArea {
                anchors.fill: parent
                onClicked: ladder.symbolSelected(model.symbol)
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                spacing: 8

                Text {
                    text: model.venue.toUpperCase()
                    color: Theme.accent
                    font.pixelSize: 11
                    font.bold: true
                    Layout.preferredWidth: 58
                }
                Text {
                    text: model.symbol
                    color: Theme.textPrimary
                    font.pixelSize: 12
                    font.bold: true
                    Layout.fillWidth: true
                }
                Text {
                    text: ladder.fmt ? ladder.fmt.formatDecimal(model.bid) : model.bid
                    color: Theme.positive
                    font.pixelSize: 12
                    font.family: Theme.monoFont
                    Layout.preferredWidth: 96
                    horizontalAlignment: Text.AlignRight
                }
                Text {
                    text: ladder.fmt ? ladder.fmt.formatDecimal(model.ask) : model.ask
                    color: Theme.negative
                    font.pixelSize: 12
                    font.family: Theme.monoFont
                    Layout.preferredWidth: 96
                    horizontalAlignment: Text.AlignRight
                }
                Text {
                    text: ladder.fmt ? ladder.fmt.formatDecimal(model.last) : model.last
                    color: Theme.textPrimary
                    font.pixelSize: 12
                    font.family: Theme.monoFont
                    Layout.preferredWidth: 96
                    horizontalAlignment: Text.AlignRight
                }
                Text {
                    text: ladder.fmt ? ladder.fmt.formatDecimal(model.spreadBps, 2) : model.spreadBps
                    color: Theme.textSecondary
                    font.pixelSize: 12
                    font.family: Theme.monoFont
                    Layout.preferredWidth: 88
                    horizontalAlignment: Text.AlignRight
                }
            }
        }
    }

    Text {
        visible: !ladder.ticksModel || ladder.ticksModel.count === 0
        text: "No market ticks received yet."
        color: Theme.textSecondary
        font.pixelSize: 12
    }
}
