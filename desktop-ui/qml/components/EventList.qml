import QtQuick
import QtQuick.Layouts
import BinanceBot

PanelCard {
    id: eventCard
    title: "RECENT EVENTS"

    property var eventsModel
    property string levelFilter: "ALL"
    property string searchQuery: ""
    property int preferredListHeight: 160

    ListView {
        id: listView
        Layout.fillWidth: true
        Layout.preferredHeight: eventCard.preferredListHeight
        Layout.fillHeight: true
        clip: true
        spacing: 2
        model: eventCard.eventsModel

        delegate: Item {
            readonly property bool matchesLevel: eventCard.levelFilter === "ALL" || model.level === eventCard.levelFilter
            readonly property bool matchesQuery: eventCard.searchQuery.length === 0
                || model.message.toLowerCase().indexOf(eventCard.searchQuery.toLowerCase()) >= 0
                || model.kind.toLowerCase().indexOf(eventCard.searchQuery.toLowerCase()) >= 0
                || model.symbol.toLowerCase().indexOf(eventCard.searchQuery.toLowerCase()) >= 0
            readonly property bool shown: matchesLevel && matchesQuery

            width: listView.width
            height: shown ? 26 : 0
            visible: shown

            RowLayout {
                anchors.fill: parent
                spacing: 8

                Text {
                    text: "[" + model.level.toUpperCase() + "]"
                    color: Theme.levelColor(model.level)
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.monoFont
                    Layout.preferredWidth: 74
                }

                Text {
                    visible: model.venue.length > 0
                    text: model.venue.toUpperCase()
                    color: Theme.accent
                    font.pixelSize: 11
                    font.bold: true
                    Layout.preferredWidth: 46
                }

                Text {
                    text: model.kind + (model.symbol.length > 0 ? (" · " + model.symbol) : "")
                    color: Theme.textPrimary
                    font.pixelSize: 12
                    font.bold: true
                    Layout.preferredWidth: 150
                    elide: Text.ElideRight
                }

                Text {
                    text: model.message
                    color: Theme.textSecondary
                    font.pixelSize: 12
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }
            }
        }
    }

    Text {
        visible: !eventCard.eventsModel || eventCard.eventsModel.count === 0
        text: "No events recorded in current session."
        color: Theme.textSecondary
        font.pixelSize: 12
    }
}
